//! Cgroup Memory and CPU Controllers - limits, usage tracking, OOM,
//! hierarchical accounting, shares, quota/period, throttling, burst.

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Cgroup Memory Controller
// ---------------------------------------------------------------------------

/// Memory statistics counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryStat {
    /// Resident set size in bytes.
    pub rss: u64,
    /// Page cache usage in bytes.
    pub cache: u64,
    /// Memory-mapped file usage in bytes.
    pub mapped_file: u64,
    /// Anonymous memory usage in bytes.
    pub anon: u64,
    /// Swap usage in bytes.
    pub swap: u64,
}

impl MemoryStat {
    /// Total memory usage (rss + cache).
    pub fn total(&self) -> u64 {
        self.rss.saturating_add(self.cache)
    }
}

/// OOM event information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OomEvent {
    /// Number of OOM events triggered.
    pub oom_kill_count: u64,
    /// Whether OOM kill is enabled.
    pub oom_kill_enabled: bool,
    /// Whether the group is currently under OOM.
    pub under_oom: bool,
}

impl Default for OomEvent {
    fn default() -> Self {
        Self {
            oom_kill_count: 0,
            oom_kill_enabled: true,
            under_oom: false,
        }
    }
}

/// Cgroup memory controller.
#[derive(Debug, Clone)]
pub struct CgroupMemoryController {
    /// Hard memory limit in bytes (0 = unlimited).
    pub limit_hard: u64,
    /// Soft memory limit in bytes (0 = unlimited).
    pub limit_soft: u64,
    /// Current usage in bytes.
    pub usage_current: u64,
    /// Peak (maximum) usage in bytes.
    pub usage_peak: u64,
    /// Detailed memory statistics.
    pub stat: MemoryStat,
    /// OOM event state.
    pub oom: OomEvent,
    /// Parent cgroup ID for hierarchical accounting (0 = root).
    pub parent_id: u64,
    /// Unique cgroup ID.
    pub cgroup_id: u64,
}

impl CgroupMemoryController {
    pub fn new(cgroup_id: u64) -> Self {
        Self {
            limit_hard: 0,
            limit_soft: 0,
            usage_current: 0,
            usage_peak: 0,
            stat: MemoryStat::default(),
            oom: OomEvent::default(),
            parent_id: 0,
            cgroup_id,
        }
    }

    /// Set the hard limit. Returns error if current usage exceeds new limit.
    pub fn set_hard_limit(&mut self, limit: u64) -> Result<(), KernelError> {
        if limit > 0 && self.usage_current > limit {
            // Trigger reclaim attempt
            self.try_reclaim(self.usage_current.saturating_sub(limit));
            if self.usage_current > limit {
                return Err(KernelError::ResourceExhausted {
                    resource: "cgroup memory",
                });
            }
        }
        self.limit_hard = limit;
        Ok(())
    }

    /// Set the soft limit.
    pub fn set_soft_limit(&mut self, limit: u64) {
        self.limit_soft = limit;
    }

    /// Charge memory usage. Returns error if hard limit would be exceeded.
    pub fn charge(&mut self, bytes: u64) -> Result<(), KernelError> {
        let new_usage = self.usage_current.saturating_add(bytes);
        if self.limit_hard > 0 && new_usage > self.limit_hard {
            // Try reclaim first
            self.try_reclaim(new_usage.saturating_sub(self.limit_hard));
            let after_reclaim = self.usage_current.saturating_add(bytes);
            if after_reclaim > self.limit_hard {
                self.oom.under_oom = true;
                self.oom.oom_kill_count = self.oom.oom_kill_count.saturating_add(1);
                return Err(KernelError::OutOfMemory {
                    requested: bytes as usize,
                    available: self.limit_hard.saturating_sub(self.usage_current) as usize,
                });
            }
        }
        self.usage_current = self.usage_current.saturating_add(bytes);
        if self.usage_current > self.usage_peak {
            self.usage_peak = self.usage_current;
        }
        self.stat.rss = self.stat.rss.saturating_add(bytes);
        Ok(())
    }

    /// Uncharge (release) memory usage.
    pub fn uncharge(&mut self, bytes: u64) {
        self.usage_current = self.usage_current.saturating_sub(bytes);
        self.stat.rss = self.stat.rss.saturating_sub(bytes);
        self.oom.under_oom = false;
    }

    /// Check if soft limit is exceeded (triggers reclaim pressure).
    pub fn soft_limit_exceeded(&self) -> bool {
        self.limit_soft > 0 && self.usage_current > self.limit_soft
    }

    /// Try to reclaim `target` bytes. Returns bytes reclaimed.
    /// In a real implementation this would trigger page reclaim; here it
    /// reclaims from cache.
    fn try_reclaim(&mut self, target: u64) -> u64 {
        let reclaimable = self.stat.cache;
        let reclaimed = if reclaimable >= target {
            target
        } else {
            reclaimable
        };
        self.stat.cache = self.stat.cache.saturating_sub(reclaimed);
        self.usage_current = self.usage_current.saturating_sub(reclaimed);
        reclaimed
    }

    /// Record a cache page addition.
    pub fn add_cache(&mut self, bytes: u64) {
        self.stat.cache = self.stat.cache.saturating_add(bytes);
        self.usage_current = self.usage_current.saturating_add(bytes);
        if self.usage_current > self.usage_peak {
            self.usage_peak = self.usage_current;
        }
    }

    /// Record a mapped file addition.
    pub fn add_mapped_file(&mut self, bytes: u64) {
        self.stat.mapped_file = self.stat.mapped_file.saturating_add(bytes);
    }

    /// Hierarchical usage including parent chain (simplified: just self).
    pub fn hierarchical_usage(&self) -> u64 {
        self.usage_current
    }
}

// ---------------------------------------------------------------------------
// Cgroup CPU Controller
// ---------------------------------------------------------------------------

/// CPU bandwidth statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuBandwidthStats {
    /// Number of times throttled.
    pub nr_throttled: u64,
    /// Total throttled time in nanoseconds.
    pub throttled_time_ns: u64,
    /// Number of scheduling periods elapsed.
    pub nr_periods: u64,
    /// Burst time accumulated in nanoseconds.
    pub nr_bursts: u64,
    /// Total burst time used in nanoseconds.
    pub burst_time_ns: u64,
}

/// Cgroup CPU controller with shares and bandwidth limiting.
#[derive(Debug, Clone)]
pub struct CgroupCpuController {
    /// CPU shares (weight-based fair scheduling, default 1024).
    pub shares: u32,
    /// CPU quota in microseconds per period (0 = unlimited).
    pub quota_us: u64,
    /// CPU period in microseconds (default 100000 = 100ms).
    pub period_us: u64,
    /// Burst capacity in microseconds (0 = no burst).
    pub burst_us: u64,
    /// Accumulated burst budget in nanoseconds.
    pub(crate) burst_budget_ns: u64,
    /// Runtime consumed in the current period in nanoseconds.
    runtime_consumed_ns: u64,
    /// Bandwidth statistics.
    pub stats: CpuBandwidthStats,
    /// Whether currently throttled.
    pub throttled: bool,
    /// Parent cgroup ID for hierarchical distribution (0 = root).
    pub parent_id: u64,
    /// Unique cgroup ID.
    pub cgroup_id: u64,
}

impl CgroupCpuController {
    pub fn new(cgroup_id: u64) -> Self {
        Self {
            shares: 1024,
            quota_us: 0,
            period_us: 100_000,
            burst_us: 0,
            burst_budget_ns: 0,
            runtime_consumed_ns: 0,
            stats: CpuBandwidthStats::default(),
            throttled: false,
            parent_id: 0,
            cgroup_id,
        }
    }

    /// Set CPU shares (weight). Minimum 2, maximum 262144.
    pub fn set_shares(&mut self, shares: u32) -> Result<(), KernelError> {
        if !(2..=262144).contains(&shares) {
            return Err(KernelError::InvalidArgument {
                name: "cpu.shares",
                value: "out of range [2, 262144]",
            });
        }
        self.shares = shares;
        Ok(())
    }

    /// Set CPU bandwidth quota and period.
    /// quota_us=0 means unlimited. Period must be >= 1000us and <= 1000000us.
    pub fn set_bandwidth(&mut self, quota_us: u64, period_us: u64) -> Result<(), KernelError> {
        if !(1000..=1_000_000).contains(&period_us) {
            return Err(KernelError::InvalidArgument {
                name: "cpu.cfs_period_us",
                value: "out of range [1000, 1000000]",
            });
        }
        if quota_us > 0 && quota_us < 1000 {
            return Err(KernelError::InvalidArgument {
                name: "cpu.cfs_quota_us",
                value: "must be >= 1000 or 0 (unlimited)",
            });
        }
        self.quota_us = quota_us;
        self.period_us = period_us;
        Ok(())
    }

    /// Set burst capacity in microseconds.
    pub fn set_burst(&mut self, burst_us: u64) {
        self.burst_us = burst_us;
    }

    /// Consume runtime. Returns true if the task is now throttled.
    pub fn consume_runtime(&mut self, ns: u64) -> bool {
        self.runtime_consumed_ns = self.runtime_consumed_ns.saturating_add(ns);

        if self.quota_us == 0 {
            return false; // unlimited
        }

        // Convert quota from us to ns: quota_us * 1000
        let quota_ns = self.quota_us.saturating_mul(1000);
        let effective_quota = quota_ns.saturating_add(self.burst_budget_ns);

        if self.runtime_consumed_ns > effective_quota {
            self.throttled = true;
            self.stats.nr_throttled = self.stats.nr_throttled.saturating_add(1);
            let overshoot = self.runtime_consumed_ns.saturating_sub(effective_quota);
            self.stats.throttled_time_ns = self.stats.throttled_time_ns.saturating_add(overshoot);
            true
        } else {
            false
        }
    }

    /// Begin a new scheduling period. Refills runtime and handles burst.
    pub fn new_period(&mut self) {
        self.stats.nr_periods = self.stats.nr_periods.saturating_add(1);

        if self.quota_us > 0 {
            let quota_ns = self.quota_us.saturating_mul(1000);
            // Any unused runtime becomes burst budget (up to burst limit)
            if self.runtime_consumed_ns < quota_ns {
                let unused = quota_ns.saturating_sub(self.runtime_consumed_ns);
                let burst_limit_ns = self.burst_us.saturating_mul(1000);
                self.burst_budget_ns = self
                    .burst_budget_ns
                    .saturating_add(unused)
                    .min(burst_limit_ns);
                if unused > 0 {
                    self.stats.nr_bursts = self.stats.nr_bursts.saturating_add(1);
                    self.stats.burst_time_ns = self.stats.burst_time_ns.saturating_add(unused);
                }
            } else {
                // Used from burst budget
                let overdraft = self.runtime_consumed_ns.saturating_sub(quota_ns);
                self.burst_budget_ns = self.burst_budget_ns.saturating_sub(overdraft);
            }
        }

        self.runtime_consumed_ns = 0;
        self.throttled = false;
    }

    /// Calculate the effective CPU percentage (quota/period * 100).
    /// Returns percentage * 100 (fixed-point with 2 decimal digits).
    /// For example, quota=50000, period=100000 returns 5000 (50.00%).
    pub fn effective_cpu_percent_x100(&self) -> u64 {
        if self.quota_us == 0 || self.period_us == 0 {
            return 0; // unlimited or invalid
        }
        // (quota_us * 10000) / period_us gives percent * 100
        self.quota_us
            .saturating_mul(10000)
            .checked_div(self.period_us)
            .unwrap_or(0)
    }

    /// Compute the weight-proportional share of CPU time for this cgroup
    /// relative to a total weight sum. Returns nanoseconds per period.
    pub fn proportional_runtime_ns(&self, total_shares: u32) -> u64 {
        if total_shares == 0 {
            return 0;
        }
        let period_ns = self.period_us.saturating_mul(1000);
        // (shares * period_ns) / total_shares
        let shares_u64 = self.shares as u64;
        shares_u64
            .saturating_mul(period_ns)
            .checked_div(total_shares as u64)
            .unwrap_or(0)
    }
}
