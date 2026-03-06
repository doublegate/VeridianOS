//! Crontab Scheduler
//!
//! Implements periodic job scheduling with standard crontab syntax,
//! per-user crontabs, and a cron daemon for job execution.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

use super::helpers::{parse_u8, push_u32_str};

// ============================================================================
// Constants
// ============================================================================

/// Maximum crontab entries per user
const MAX_CRON_ENTRIES: usize = 256;

// ============================================================================
// Error Types
// ============================================================================

/// Crontab errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronError {
    /// Invalid cron expression
    InvalidExpression,
    /// Invalid field value
    InvalidField,
    /// Value out of range
    OutOfRange,
    /// Too many entries
    TooManyEntries,
    /// Entry not found
    NotFound,
    /// User not permitted
    PermissionDenied,
    /// Parse error
    ParseError,
}

// ============================================================================
// Cron Field
// ============================================================================

/// Cron field specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronField {
    /// Match any value (*)
    Any,
    /// Match a specific value
    Value(u8),
    /// Match a range (start-end inclusive)
    Range(u8, u8),
    /// Match with step (*/step or start-end/step)
    Step {
        /// Start value (0 for *)
        start: u8,
        /// End value (max for *)
        end: u8,
        /// Step interval
        step: u8,
    },
    /// Match a list of values
    List(Vec<CronFieldItem>),
}

/// A single item in a cron field list
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronFieldItem {
    /// Single value
    Value(u8),
    /// Range
    Range(u8, u8),
    /// Step
    Step { start: u8, end: u8, step: u8 },
}

impl CronField {
    /// Parse a cron field string with min/max bounds
    pub fn parse(field: &str, min: u8, max: u8) -> Result<Self, CronError> {
        // Handle list (comma-separated)
        if field.contains(',') {
            let items: Result<Vec<CronFieldItem>, CronError> = field
                .split(',')
                .map(|part| Self::parse_item(part.trim(), min, max))
                .collect();
            return Ok(CronField::List(items?));
        }

        // Handle step (*/N or start-end/N)
        if field.contains('/') {
            let parts: Vec<&str> = field.splitn(2, '/').collect();
            if parts.len() != 2 {
                return Err(CronError::InvalidField);
            }
            let step = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if step == 0 {
                return Err(CronError::InvalidField);
            }
            if parts[0] == "*" {
                return Ok(CronField::Step {
                    start: min,
                    end: max,
                    step,
                });
            }
            if parts[0].contains('-') {
                let range_parts: Vec<&str> = parts[0].splitn(2, '-').collect();
                let start = parse_u8(range_parts[0]).ok_or(CronError::InvalidField)?;
                let end = parse_u8(range_parts[1]).ok_or(CronError::InvalidField)?;
                if start > end || start < min || end > max {
                    return Err(CronError::OutOfRange);
                }
                return Ok(CronField::Step { start, end, step });
            }
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            return Ok(CronField::Step {
                start,
                end: max,
                step,
            });
        }

        // Handle wildcard
        if field == "*" {
            return Ok(CronField::Any);
        }

        // Handle range (start-end)
        if field.contains('-') {
            let parts: Vec<&str> = field.splitn(2, '-').collect();
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            let end = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if start > end || start < min || end > max {
                return Err(CronError::OutOfRange);
            }
            return Ok(CronField::Range(start, end));
        }

        // Single value
        let val = parse_u8(field).ok_or(CronError::InvalidField)?;
        if val < min || val > max {
            return Err(CronError::OutOfRange);
        }
        Ok(CronField::Value(val))
    }

    /// Parse a single item (for list parsing)
    fn parse_item(item: &str, min: u8, max: u8) -> Result<CronFieldItem, CronError> {
        if item.contains('/') {
            let parts: Vec<&str> = item.splitn(2, '/').collect();
            let step = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if parts[0].contains('-') {
                let rp: Vec<&str> = parts[0].splitn(2, '-').collect();
                let start = parse_u8(rp[0]).ok_or(CronError::InvalidField)?;
                let end = parse_u8(rp[1]).ok_or(CronError::InvalidField)?;
                if start < min || end > max {
                    return Err(CronError::OutOfRange);
                }
                return Ok(CronFieldItem::Step { start, end, step });
            }
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            return Ok(CronFieldItem::Step {
                start,
                end: max,
                step,
            });
        }
        if item.contains('-') {
            let parts: Vec<&str> = item.splitn(2, '-').collect();
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            let end = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if start < min || end > max {
                return Err(CronError::OutOfRange);
            }
            return Ok(CronFieldItem::Range(start, end));
        }
        let val = parse_u8(item).ok_or(CronError::InvalidField)?;
        if val < min || val > max {
            return Err(CronError::OutOfRange);
        }
        Ok(CronFieldItem::Value(val))
    }

    /// Check if a given value matches this field
    pub fn matches(&self, value: u8) -> bool {
        match self {
            CronField::Any => true,
            CronField::Value(v) => value == *v,
            CronField::Range(start, end) => ((*start)..=(*end)).contains(&value),
            CronField::Step { start, end, step } => {
                if value < *start || value > *end {
                    return false;
                }
                let offset = value - *start;
                offset.is_multiple_of(*step)
            }
            CronField::List(items) => items.iter().any(|item| match item {
                CronFieldItem::Value(v) => value == *v,
                CronFieldItem::Range(s, e) => ((*s)..=(*e)).contains(&value),
                CronFieldItem::Step { start, end, step } => {
                    if value < *start || value > *end {
                        return false;
                    }
                    let offset = value - *start;
                    offset.is_multiple_of(*step)
                }
            }),
        }
    }
}

// ============================================================================
// Special Schedules
// ============================================================================

/// Special cron schedule strings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronSpecial {
    /// Run once at startup
    Reboot,
    /// Run once per year (0 0 1 1 *)
    Yearly,
    /// Run once per month (0 0 1 * *)
    Monthly,
    /// Run once per week (0 0 * * 0)
    Weekly,
    /// Run once per day (0 0 * * *)
    Daily,
    /// Run once per hour (0 * * * *)
    Hourly,
}

impl CronSpecial {
    /// Parse a special string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "@reboot" => Some(Self::Reboot),
            "@yearly" | "@annually" => Some(Self::Yearly),
            "@monthly" => Some(Self::Monthly),
            "@weekly" => Some(Self::Weekly),
            "@daily" | "@midnight" => Some(Self::Daily),
            "@hourly" => Some(Self::Hourly),
            _ => None,
        }
    }

    /// Convert to cron schedule fields
    pub fn to_schedule(self) -> CronSchedule {
        match self {
            Self::Reboot => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: true,
            },
            Self::Yearly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Value(1),
                month: CronField::Value(1),
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Monthly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Value(1),
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Weekly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Value(0),
                is_reboot: false,
            },
            Self::Daily => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Hourly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Any,
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
        }
    }
}

// ============================================================================
// Cron Schedule
// ============================================================================

/// Cron schedule (5-field specification)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    /// Minute (0-59)
    pub minute: CronField,
    /// Hour (0-23)
    pub hour: CronField,
    /// Day of month (1-31)
    pub day_of_month: CronField,
    /// Month (1-12)
    pub month: CronField,
    /// Day of week (0-6, 0=Sunday)
    pub day_of_week: CronField,
    /// Is this a @reboot schedule
    pub is_reboot: bool,
}

impl CronSchedule {
    /// Parse a 5-field cron expression or special string
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let trimmed = expr.trim();

        // Check for special strings
        if trimmed.starts_with('@') {
            return CronSpecial::from_str(trimmed)
                .map(|s| s.to_schedule())
                .ok_or(CronError::InvalidExpression);
        }

        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 5 {
            return Err(CronError::InvalidExpression);
        }

        Ok(Self {
            minute: CronField::parse(fields[0], 0, 59)?,
            hour: CronField::parse(fields[1], 0, 23)?,
            day_of_month: CronField::parse(fields[2], 1, 31)?,
            month: CronField::parse(fields[3], 1, 12)?,
            day_of_week: CronField::parse(fields[4], 0, 6)?,
            is_reboot: false,
        })
    }

    /// Check if this schedule matches a given date/time
    ///
    /// Parameters are plain integers (no floating point):
    /// - minute: 0-59
    /// - hour: 0-23
    /// - day: 1-31
    /// - month: 1-12
    /// - dow: 0-6 (0=Sunday)
    pub fn matches(&self, minute: u8, hour: u8, day: u8, month: u8, dow: u8) -> bool {
        if self.is_reboot {
            return false; // @reboot only runs at boot
        }
        self.minute.matches(minute)
            && self.hour.matches(hour)
            && self.day_of_month.matches(day)
            && self.month.matches(month)
            && self.day_of_week.matches(dow)
    }

    /// Calculate the next matching time from the given start
    ///
    /// Returns (minute, hour, day, month, year) or None if not found
    /// within a reasonable search window (1 year).
    ///
    /// All arithmetic is integer-only.
    pub fn next_run(
        &self,
        start_minute: u8,
        start_hour: u8,
        start_day: u8,
        start_month: u8,
        start_year: u16,
    ) -> Option<(u8, u8, u8, u8, u16)> {
        if self.is_reboot {
            return None;
        }

        let mut minute = start_minute;
        let mut hour = start_hour;
        let mut day = start_day;
        let mut month = start_month;
        let mut year = start_year;

        // Advance minute by 1 to avoid matching current time
        minute += 1;
        if minute > 59 {
            minute = 0;
            hour += 1;
        }
        if hour > 23 {
            hour = 0;
            day += 1;
        }

        // Search up to ~366 days * 24 hours * 60 minutes = 527040 iterations max
        // But we optimize by skipping non-matching months/days
        let max_iterations = 527_040u32;
        let mut iterations = 0u32;

        loop {
            if iterations >= max_iterations {
                return None;
            }
            iterations += 1;

            // Fix day overflow
            let max_day = days_in_month(month, year);
            if day > max_day {
                day = 1;
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                minute = 0;
                hour = 0;
                continue;
            }

            // Check month
            if !self.month.matches(month) {
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                day = 1;
                hour = 0;
                minute = 0;
                continue;
            }

            // Check day of month and day of week
            let dow = day_of_week(year, month, day);
            if !self.day_of_month.matches(day) || !self.day_of_week.matches(dow) {
                day += 1;
                hour = 0;
                minute = 0;
                continue;
            }

            // Check hour
            if !self.hour.matches(hour) {
                hour += 1;
                if hour > 23 {
                    hour = 0;
                    day += 1;
                }
                minute = 0;
                continue;
            }

            // Check minute
            if !self.minute.matches(minute) {
                minute += 1;
                if minute > 59 {
                    minute = 0;
                    hour += 1;
                    if hour > 23 {
                        hour = 0;
                        day += 1;
                    }
                }
                continue;
            }

            return Some((minute, hour, day, month, year));
        }
    }
}

// ============================================================================
// Cron Entry
// ============================================================================

/// Cron job entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEntry {
    /// Unique job ID
    pub id: u64,
    /// Schedule specification
    pub schedule: CronSchedule,
    /// Command to execute
    pub command: String,
    /// Owner username
    pub owner: String,
    /// Whether this job is enabled
    pub enabled: bool,
    /// Last run timestamp (epoch seconds)
    pub last_run: u64,
    /// Run count
    pub run_count: u64,
    /// Last exit status
    pub last_exit_status: i32,
}

impl CronEntry {
    /// Create a new cron entry
    pub fn new(id: u64, schedule: CronSchedule, command: &str, owner: &str) -> Self {
        Self {
            id,
            schedule,
            command: String::from(command),
            owner: String::from(owner),
            enabled: true,
            last_run: 0,
            run_count: 0,
            last_exit_status: 0,
        }
    }

    /// Parse a crontab line (schedule + command)
    pub fn parse(id: u64, line: &str, owner: &str) -> Result<Self, CronError> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Err(CronError::ParseError);
        }

        // Check for special strings
        if trimmed.starts_with('@') {
            let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
            if parts.len() < 2 {
                return Err(CronError::InvalidExpression);
            }
            let schedule = CronSchedule::parse(parts[0])?;
            return Ok(Self::new(id, schedule, parts[1].trim(), owner));
        }

        // Standard 5-field format: min hour dom mon dow command
        let fields: Vec<&str> = trimmed.splitn(6, char::is_whitespace).collect();
        if fields.len() < 6 {
            return Err(CronError::InvalidExpression);
        }

        let schedule_str = &[fields[0], fields[1], fields[2], fields[3], fields[4]].join(" ");
        let schedule = CronSchedule::parse(schedule_str)?;
        let command = fields[5].trim();

        Ok(Self::new(id, schedule, command, owner))
    }

    /// Serialize to crontab format
    pub fn to_crontab_line(&self) -> String {
        if self.schedule.is_reboot {
            let mut line = String::from("@reboot ");
            line.push_str(&self.command);
            return line;
        }
        let mut line = String::new();
        line.push_str(&format_cron_field(&self.schedule.minute));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.hour));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.day_of_month));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.month));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.day_of_week));
        line.push(' ');
        line.push_str(&self.command);
        line
    }
}

// ============================================================================
// CronTab
// ============================================================================

/// Per-user crontab
#[derive(Debug)]
pub struct CronTab {
    /// Owner username
    pub owner: String,
    /// Job entries
    pub entries: Vec<CronEntry>,
    /// Next entry ID
    next_id: u64,
}

impl CronTab {
    /// Create a new crontab for a user
    pub fn new(owner: &str) -> Self {
        Self {
            owner: String::from(owner),
            entries: Vec::new(),
            next_id: 1,
        }
    }

    /// Add an entry from a crontab line
    pub fn add_line(&mut self, line: &str) -> Result<u64, CronError> {
        if self.entries.len() >= MAX_CRON_ENTRIES {
            return Err(CronError::TooManyEntries);
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = CronEntry::parse(id, line, &self.owner)?;
        self.entries.push(entry);
        Ok(id)
    }

    /// Add an entry with a parsed schedule
    pub fn add_entry(&mut self, schedule: CronSchedule, command: &str) -> Result<u64, CronError> {
        if self.entries.len() >= MAX_CRON_ENTRIES {
            return Err(CronError::TooManyEntries);
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = CronEntry::new(id, schedule, command, &self.owner);
        self.entries.push(entry);
        Ok(id)
    }

    /// Remove an entry by ID
    pub fn remove_entry(&mut self, id: u64) -> Result<(), CronError> {
        let initial = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == initial {
            return Err(CronError::NotFound);
        }
        Ok(())
    }

    /// Enable/disable an entry
    pub fn set_enabled(&mut self, id: u64, enabled: bool) -> Result<(), CronError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(CronError::NotFound)?;
        entry.enabled = enabled;
        Ok(())
    }

    /// Get entries that match the current time
    pub fn get_due_entries(
        &self,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        dow: u8,
    ) -> Vec<&CronEntry> {
        self.entries
            .iter()
            .filter(|e| e.enabled && e.schedule.matches(minute, hour, day, month, dow))
            .collect()
    }

    /// Parse a complete crontab file
    pub fn load(&mut self, content: &str) -> Result<usize, CronError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Skip environment variable assignments
            if trimmed.contains('=') && !trimmed.starts_with('*') && !trimmed.starts_with('@') {
                // Could be VAR=value
                let first_char = trimmed.as_bytes()[0];
                if first_char.is_ascii_alphabetic() {
                    continue;
                }
            }
            self.add_line(trimmed)?;
            count += 1;
        }
        Ok(count)
    }

    /// Serialize to crontab file format
    pub fn to_crontab_file(&self) -> String {
        let mut output = String::new();
        output.push_str("# Crontab for ");
        output.push_str(&self.owner);
        output.push('\n');
        for entry in &self.entries {
            if !entry.enabled {
                output.push_str("# ");
            }
            output.push_str(&entry.to_crontab_line());
            output.push('\n');
        }
        output
    }

    /// Number of entries
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// ============================================================================
// Cron Job Execution
// ============================================================================

/// Cron job execution record
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobExecution {
    /// Job ID
    pub job_id: u64,
    /// Owner
    pub owner: String,
    /// Command
    pub command: String,
    /// Scheduled time (epoch seconds)
    pub scheduled_time: u64,
}

// ============================================================================
// Cron Daemon
// ============================================================================

/// Cron daemon
#[derive(Debug)]
pub struct CronDaemon {
    /// Per-user crontabs
    pub crontabs: BTreeMap<String, CronTab>,
    /// System crontab (/etc/crontab)
    pub system_crontab: CronTab,
    /// Pending execution queue
    pub execution_queue: Vec<CronJobExecution>,
    /// Total jobs executed
    pub total_executed: u64,
    /// Last tick time (for deduplication)
    pub last_tick_minute: u8,
    /// Whether the daemon is running
    pub running: bool,
    /// Whether reboot jobs have been fired
    pub reboot_fired: bool,
}

impl Default for CronDaemon {
    fn default() -> Self {
        Self::new()
    }
}

impl CronDaemon {
    /// Create a new cron daemon
    pub fn new() -> Self {
        Self {
            crontabs: BTreeMap::new(),
            system_crontab: CronTab::new("root"),
            execution_queue: Vec::new(),
            total_executed: 0,
            last_tick_minute: 255, // invalid, forces first tick
            running: false,
            reboot_fired: false,
        }
    }

    /// Start the daemon
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Stop the daemon
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Get or create a user's crontab
    pub fn get_or_create_crontab(&mut self, username: &str) -> &mut CronTab {
        if !self.crontabs.contains_key(username) {
            self.crontabs
                .insert(String::from(username), CronTab::new(username));
        }
        self.crontabs.get_mut(username).unwrap()
    }

    /// Remove a user's crontab
    pub fn remove_crontab(&mut self, username: &str) -> bool {
        self.crontabs.remove(username).is_some()
    }

    /// Fire @reboot jobs (called once at boot)
    pub fn fire_reboot_jobs(&mut self, current_time: u64) {
        if self.reboot_fired {
            return;
        }
        self.reboot_fired = true;

        // System crontab
        for entry in &self.system_crontab.entries {
            if entry.enabled && entry.schedule.is_reboot {
                self.execution_queue.push(CronJobExecution {
                    job_id: entry.id,
                    owner: entry.owner.clone(),
                    command: entry.command.clone(),
                    scheduled_time: current_time,
                });
            }
        }

        // User crontabs
        for crontab in self.crontabs.values() {
            for entry in &crontab.entries {
                if entry.enabled && entry.schedule.is_reboot {
                    self.execution_queue.push(CronJobExecution {
                        job_id: entry.id,
                        owner: entry.owner.clone(),
                        command: entry.command.clone(),
                        scheduled_time: current_time,
                    });
                }
            }
        }
    }

    /// Tick the daemon with current time components
    ///
    /// Should be called once per minute. Enqueues any matching jobs.
    pub fn tick(
        &mut self,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        dow: u8,
        current_time: u64,
    ) -> usize {
        if !self.running {
            return 0;
        }

        // Deduplicate: only process each minute once
        if minute == self.last_tick_minute {
            return 0;
        }
        self.last_tick_minute = minute;

        let mut queued = 0usize;

        // Check system crontab
        let due_system: Vec<(u64, String, String)> = self
            .system_crontab
            .get_due_entries(minute, hour, day, month, dow)
            .iter()
            .map(|e| (e.id, e.owner.clone(), e.command.clone()))
            .collect();

        for (id, owner, cmd) in due_system {
            self.execution_queue.push(CronJobExecution {
                job_id: id,
                owner,
                command: cmd,
                scheduled_time: current_time,
            });
            queued += 1;
        }

        // Check user crontabs
        let user_due: Vec<(u64, String, String)> = self
            .crontabs
            .values()
            .flat_map(|tab| {
                tab.get_due_entries(minute, hour, day, month, dow)
                    .into_iter()
                    .map(|e| (e.id, e.owner.clone(), e.command.clone()))
            })
            .collect();

        for (id, owner, cmd) in user_due {
            self.execution_queue.push(CronJobExecution {
                job_id: id,
                owner,
                command: cmd,
                scheduled_time: current_time,
            });
            queued += 1;
        }

        self.total_executed += queued as u64;
        queued
    }

    /// Drain the execution queue
    pub fn drain_queue(&mut self) -> Vec<CronJobExecution> {
        core::mem::take(&mut self.execution_queue)
    }

    /// Get the total number of crontab entries across all users
    pub fn total_entries(&self) -> usize {
        let user_entries: usize = self.crontabs.values().map(|t| t.entry_count()).sum();
        user_entries + self.system_crontab.entry_count()
    }

    /// Check if the daemon is running
    pub fn is_running(&self) -> bool {
        self.running
    }
}

// ============================================================================
// Helper Functions (cron-specific)
// ============================================================================

/// Days in a given month (integer arithmetic, no floating point)
pub(crate) fn days_in_month(month: u8, year: u16) -> u8 {
    match month {
        1 => 31,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        3 => 31,
        4 => 30,
        5 => 31,
        6 => 30,
        7 => 31,
        8 => 31,
        9 => 30,
        10 => 31,
        11 => 30,
        12 => 31,
        _ => 30,
    }
}

/// Check if a year is a leap year (integer-only)
pub(crate) fn is_leap_year(year: u16) -> bool {
    if year.is_multiple_of(400) {
        true
    } else if year.is_multiple_of(100) {
        false
    } else {
        year.is_multiple_of(4)
    }
}

/// Calculate day of week (0=Sunday) using Tomohiko Sakamoto's algorithm
/// (integer-only)
pub(crate) fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if month < 3 {
        y -= 1;
    }
    let m = month as u16;
    let d = day as u16;
    ((y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + d) % 7) as u8
}

/// Format a cron field as a string
pub(crate) fn format_cron_field(field: &CronField) -> String {
    match field {
        CronField::Any => String::from("*"),
        CronField::Value(v) => {
            let mut s = String::new();
            push_u32_str(&mut s, *v as u32);
            s
        }
        CronField::Range(start, end) => {
            let mut s = String::new();
            push_u32_str(&mut s, *start as u32);
            s.push('-');
            push_u32_str(&mut s, *end as u32);
            s
        }
        CronField::Step { start, end, step } => {
            let mut s = String::new();
            if *start == 0 {
                s.push('*');
            } else {
                push_u32_str(&mut s, *start as u32);
                s.push('-');
                push_u32_str(&mut s, *end as u32);
            }
            s.push('/');
            push_u32_str(&mut s, *step as u32);
            s
        }
        CronField::List(items) => {
            let mut s = String::new();
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                match item {
                    CronFieldItem::Value(v) => push_u32_str(&mut s, *v as u32),
                    CronFieldItem::Range(start, end) => {
                        push_u32_str(&mut s, *start as u32);
                        s.push('-');
                        push_u32_str(&mut s, *end as u32);
                    }
                    CronFieldItem::Step { start, end, step } => {
                        push_u32_str(&mut s, *start as u32);
                        s.push('-');
                        push_u32_str(&mut s, *end as u32);
                        s.push('/');
                        push_u32_str(&mut s, *step as u32);
                    }
                }
            }
            s
        }
    }
}
