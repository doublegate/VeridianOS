//! IPC rate limiting implementation
//!
//! Provides rate limiting for IPC operations to prevent DoS attacks
//! and ensure fair resource usage.

#![allow(dead_code)]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::{
    capability::ProcessId,
    error::{IpcError, Result},
};

/// Rate limiter for IPC operations
pub struct RateLimiter {
    /// Token bucket for rate limiting
    buckets: [TokenBucket; MAX_PROCESSES],
}

/// Maximum number of processes to track
const MAX_PROCESSES: usize = 1024;

/// Token bucket for rate limiting
struct TokenBucket {
    /// Process ID this bucket belongs to
    pid: AtomicU64,
    /// Current number of tokens
    tokens: AtomicU32,
    /// Maximum tokens (bucket capacity)
    max_tokens: AtomicU32,
    /// Tokens per second refill rate
    refill_rate: AtomicU32,
    /// Last refill timestamp
    last_refill: AtomicU64,
    /// Messages sent in current window
    messages_sent: AtomicU64,
    /// Bytes sent in current window
    bytes_sent: AtomicU64,
}

impl TokenBucket {
    const fn new() -> Self {
        Self {
            pid: AtomicU64::new(0),
            tokens: AtomicU32::new(100),
            max_tokens: AtomicU32::new(100),
            refill_rate: AtomicU32::new(100),
            last_refill: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
        }
    }

    /// Try to consume tokens
    fn try_consume(&self, tokens_needed: u32) -> bool {
        // First refill tokens based on elapsed time
        self.refill();

        // Try to consume tokens
        let mut current = self.tokens.load(Ordering::Acquire);
        loop {
            if current < tokens_needed {
                return false;
            }

            match self.tokens.compare_exchange_weak(
                current,
                current - tokens_needed,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(val) => current = val,
            }
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&self) {
        let now = get_current_time();
        let last = self.last_refill.load(Ordering::Acquire);
        let elapsed_ms = (now - last) / 1_000_000; // Convert ns to ms

        if elapsed_ms > 0 {
            let refill_rate = self.refill_rate.load(Ordering::Relaxed);
            let tokens_to_add = (refill_rate as u64 * elapsed_ms / 1000) as u32;

            if tokens_to_add > 0 {
                // Update last refill time
                self.last_refill.store(now, Ordering::Release);

                // Add tokens, capping at max
                let max_tokens = self.max_tokens.load(Ordering::Relaxed);
                let mut current = self.tokens.load(Ordering::Acquire);

                loop {
                    let new_tokens = (current + tokens_to_add).min(max_tokens);
                    match self.tokens.compare_exchange_weak(
                        current,
                        new_tokens,
                        Ordering::Release,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => break,
                        Err(val) => current = val,
                    }
                }
            }
        }
    }

    /// Reset the bucket for a new process
    fn reset(&self, pid: ProcessId, max_tokens: u32, refill_rate: u32) {
        self.pid.store(pid, Ordering::Release);
        self.tokens.store(max_tokens, Ordering::Release);
        self.max_tokens.store(max_tokens, Ordering::Release);
        self.refill_rate.store(refill_rate, Ordering::Release);
        self.last_refill
            .store(get_current_time(), Ordering::Release);
        self.messages_sent.store(0, Ordering::Release);
        self.bytes_sent.store(0, Ordering::Release);
    }
}

impl RateLimiter {
    /// Create a new rate limiter
    pub const fn new() -> Self {
        // Can't use array initialization with const fn, so we'll do it manually
        #[allow(clippy::declare_interior_mutable_const)]
        const BUCKET: TokenBucket = TokenBucket::new();
        Self {
            buckets: [BUCKET; MAX_PROCESSES],
        }
    }

    /// Check if an operation is allowed
    pub fn check_allowed(
        &self,
        pid: ProcessId,
        message_size: usize,
        limits: &RateLimits,
    ) -> Result<()> {
        // Find or allocate bucket for this process
        let bucket = self.get_or_create_bucket(pid, limits)?;

        // Check message rate limit
        if limits.max_messages_per_sec > 0 {
            let tokens_needed = 1;
            if !bucket.try_consume(tokens_needed) {
                return Err(IpcError::RateLimitExceeded);
            }
        }

        // Update statistics
        bucket.messages_sent.fetch_add(1, Ordering::Relaxed);
        bucket
            .bytes_sent
            .fetch_add(message_size as u64, Ordering::Relaxed);

        // Check bandwidth limit
        if limits.max_bytes_per_sec > 0 {
            let bytes_sent = bucket.bytes_sent.load(Ordering::Relaxed);
            if bytes_sent > limits.max_bytes_per_sec {
                return Err(IpcError::RateLimitExceeded);
            }
        }

        Ok(())
    }

    /// Get or create a bucket for a process
    fn get_or_create_bucket(&self, pid: ProcessId, limits: &RateLimits) -> Result<&TokenBucket> {
        // Hash the PID to a bucket index
        let index = (pid as usize) % MAX_PROCESSES;
        let bucket = &self.buckets[index];

        // Check if this bucket is for our process
        let current_pid = bucket.pid.load(Ordering::Acquire);
        if current_pid == pid {
            return Ok(bucket);
        }

        // Try to claim this bucket
        if current_pid == 0 {
            match bucket
                .pid
                .compare_exchange(0, pid, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    // Successfully claimed, initialize it
                    bucket.reset(
                        pid,
                        limits.max_messages_per_sec,
                        limits.max_messages_per_sec,
                    );
                    return Ok(bucket);
                }
                Err(_) => {
                    // Someone else claimed it, check if it's ours now
                    if bucket.pid.load(Ordering::Acquire) == pid {
                        return Ok(bucket);
                    }
                }
            }
        }

        // Bucket collision - for now, allow the operation
        // In production, we'd implement a more sophisticated scheme
        Ok(bucket)
    }

    /// Get statistics for a process
    pub fn get_stats(&self, pid: ProcessId) -> RateLimitStats {
        let index = (pid as usize) % MAX_PROCESSES;
        let bucket = &self.buckets[index];

        if bucket.pid.load(Ordering::Acquire) == pid {
            RateLimitStats {
                messages_sent: bucket.messages_sent.load(Ordering::Relaxed),
                bytes_sent: bucket.bytes_sent.load(Ordering::Relaxed),
                tokens_available: bucket.tokens.load(Ordering::Relaxed),
                max_tokens: bucket.max_tokens.load(Ordering::Relaxed),
            }
        } else {
            RateLimitStats::default()
        }
    }
}

/// Rate limit configuration
#[derive(Debug, Clone, Copy)]
pub struct RateLimits {
    /// Maximum messages per second (0 = unlimited)
    pub max_messages_per_sec: u32,
    /// Maximum bytes per second (0 = unlimited)
    pub max_bytes_per_sec: u64,
    /// Burst capacity multiplier
    pub burst_multiplier: u32,
}

impl RateLimits {
    /// Create unlimited rate limits
    pub const fn unlimited() -> Self {
        Self {
            max_messages_per_sec: 0,
            max_bytes_per_sec: 0,
            burst_multiplier: 1,
        }
    }

    /// Create default rate limits
    pub const fn default() -> Self {
        Self {
            max_messages_per_sec: 1000,
            max_bytes_per_sec: 10 * 1024 * 1024, // 10 MB/s
            burst_multiplier: 2,
        }
    }

    /// Create strict rate limits
    pub const fn strict() -> Self {
        Self {
            max_messages_per_sec: 100,
            max_bytes_per_sec: 1024 * 1024, // 1 MB/s
            burst_multiplier: 1,
        }
    }
}

/// Rate limit statistics
#[derive(Debug, Default)]
pub struct RateLimitStats {
    pub messages_sent: u64,
    pub bytes_sent: u64,
    pub tokens_available: u32,
    pub max_tokens: u32,
}

/// Global rate limiter instance
pub static RATE_LIMITER: RateLimiter = RateLimiter::new();

/// Get current time in nanoseconds
fn get_current_time() -> u64 {
    // In a real system, this would use a high-resolution timer
    // For now, use the timestamp counter
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket() {
        let bucket = TokenBucket::new();
        bucket.reset(1, 10, 10);

        // Should be able to consume tokens
        assert!(bucket.try_consume(5));
        assert!(bucket.try_consume(5));

        // Should fail - no tokens left
        assert!(!bucket.try_consume(1));
    }

    #[test]
    fn test_rate_limiter() {
        let limits = RateLimits {
            max_messages_per_sec: 10,
            max_bytes_per_sec: 1000,
            burst_multiplier: 1,
        };

        // Should allow initial messages
        assert!(RATE_LIMITER.check_allowed(1, 100, &limits).is_ok());

        // Get stats
        let stats = RATE_LIMITER.get_stats(1);
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.bytes_sent, 100);
    }
}
