# Veridian OS API Design Guidelines

## Table of Contents

1. [Design Philosophy](#design-philosophy)
1. [Naming Conventions](#naming-conventions)
1. [Type Design](#type-design)
1. [Error Handling](#error-handling)
1. [Memory Management](#memory-management)
1. [Concurrency Patterns](#concurrency-patterns)
1. [API Stability](#api-stability)
1. [Documentation Standards](#documentation-standards)
1. [Examples](#examples)
1. [Anti-Patterns](#anti-patterns)

## Design Philosophy

### Core Principles

1. **Safety First**: APIs should be hard to misuse
1. **Clarity Over Brevity**: Clear names beat short names
1. **Consistency**: Similar operations should have similar interfaces
1. **Zero-Cost Abstractions**: Performance should not be sacrificed for ergonomics
1. **Progressive Disclosure**: Simple things should be simple, complex things possible

### Rust API Guidelines

We follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) with OS-specific extensions:

- **Type Safety**: Use the type system to enforce invariants
- **Ownership**: Make ownership and borrowing requirements clear
- **Fallibility**: Make potential failures explicit in types
- **Performance**: Document performance characteristics
- **Security**: Consider security implications of every API

## Naming Conventions

### General Rules

```rust
// Types use UpperCamelCase
pub struct ProcessId(u64);
pub enum ProcessState { Running, Blocked, Terminated }
pub trait MemoryManager { }

// Functions and methods use snake_case
pub fn create_process() -> ProcessId { }
pub fn allocate_memory(size: usize) -> *mut u8 { }

// Constants use SCREAMING_SNAKE_CASE
pub const MAX_PROCESSES: usize = 65536;
pub const PAGE_SIZE: usize = 4096;

// Module names use snake_case
mod memory_management;
mod process_scheduler;
```

### Specific Patterns

```rust
// Constructors
impl Process {
    // Primary constructor
    pub fn new(config: ProcessConfig) -> Self { }
    
    // Alternative constructors use descriptive names
    pub fn with_capabilities(caps: CapabilitySet) -> Self { }
    pub fn from_elf(elf_data: &[u8]) -> Result<Self, Error> { }
}

// Getters drop the 'get_' prefix
impl Process {
    pub fn id(&self) -> ProcessId { }           // Not get_id()
    pub fn state(&self) -> ProcessState { }     // Not get_state()
    pub fn memory_usage(&self) -> usize { }     // Not get_memory_usage()
}

// Setters use 'set_' prefix
impl Process {
    pub fn set_priority(&mut self, priority: Priority) { }
    pub fn set_affinity(&mut self, cpus: CpuSet) { }
}

// Conversion methods
impl Process {
    pub fn as_bytes(&self) -> &[u8] { }         // Cheap reference conversion
    pub fn to_string(&self) -> String { }       // Expensive conversion
    pub fn into_raw(self) -> *mut Process { }   // Consuming conversion
}
```

### Action Methods

```rust
// Boolean queries use is_, has_, can_
pub fn is_running(&self) -> bool { }
pub fn has_capability(&self, cap: Capability) -> bool { }
pub fn can_execute(&self, path: &Path) -> bool { }

// Fallible operations return Result
pub fn try_lock(&self) -> Result<Guard, TryLockError> { }
pub fn send_signal(&mut self, signal: Signal) -> Result<(), Error> { }

// Infallible operations don't use 'try_'
pub fn lock(&self) -> Guard { }  // Blocks until available
pub fn pid(&self) -> ProcessId { } // Always succeeds
```

## Type Design

### Newtype Pattern

```rust
// Use newtypes for type safety
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const INIT: Self = Self(1);
    
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

// Implement Display for user-facing output
impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PID:{}", self.0)
    }
}

// Example: Physical vs Virtual addresses
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct PhysAddr(u64);

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct VirtAddr(u64);

// Type system prevents mixing addresses
pub fn map_page(virt: VirtAddr, phys: PhysAddr) { }
```

### Builder Pattern

```rust
/// Process creation builder
pub struct ProcessBuilder {
    name: Option<String>,
    priority: Priority,
    memory_limit: Option<usize>,
    capabilities: CapabilitySet,
}

impl ProcessBuilder {
    pub fn new() -> Self {
        Self {
            name: None,
            priority: Priority::Normal,
            memory_limit: None,
            capabilities: CapabilitySet::empty(),
        }
    }
    
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
    
    pub fn memory_limit(mut self, limit: usize) -> Self {
        self.memory_limit = Some(limit);
        self
    }
    
    pub fn capability(mut self, cap: Capability) -> Self {
        self.capabilities.insert(cap);
        self
    }
    
    pub fn build(self) -> Result<Process, ProcessCreateError> {
        let name = self.name.ok_or(ProcessCreateError::MissingName)?;
        
        Ok(Process {
            name,
            priority: self.priority,
            memory_limit: self.memory_limit,
            capabilities: self.capabilities,
            ..Default::default()
        })
    }
}

// Usage
let process = ProcessBuilder::new()
    .name("init")
    .priority(Priority::High)
    .capability(Capability::Admin)
    .build()?;
```

### State Machines

```rust
/// Type-safe state machine for TCP connections
pub struct TcpConnection<S: TcpState> {
    socket: Socket,
    state: S,
}

pub trait TcpState { }

pub struct Listen;
pub struct SynReceived;
pub struct Established;
pub struct Closed;

impl TcpState for Listen { }
impl TcpState for SynReceived { }
impl TcpState for Established { }
impl TcpState for Closed { }

impl TcpConnection<Listen> {
    pub fn accept(self) -> Result<TcpConnection<SynReceived>, Error> {
        // State transition only available in Listen state
        Ok(TcpConnection {
            socket: self.socket,
            state: SynReceived,
        })
    }
}

impl TcpConnection<Established> {
    pub fn send(&mut self, data: &[u8]) -> Result<usize, Error> {
        // Send only available in Established state
        self.socket.send(data)
    }
    
    pub fn close(self) -> TcpConnection<Closed> {
        TcpConnection {
            socket: self.socket,
            state: Closed,
        }
    }
}
```

## Error Handling

### Error Types

```rust
use thiserror::Error;

/// Detailed error types with context
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("Out of memory: requested {requested} bytes, available {available} bytes")]
    OutOfMemory {
        requested: usize,
        available: usize,
    },
    
    #[error("Invalid alignment: {0} is not a power of two")]
    InvalidAlignment(usize),
    
    #[error("Address {addr:?} is not mapped")]
    UnmappedAddress { addr: VirtAddr },
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result type alias for convenience
pub type Result<T, E = MemoryError> = core::result::Result<T, E>;
```

### Error Propagation

```rust
/// Use ? operator for propagation
pub fn high_level_operation() -> Result<Data> {
    let memory = allocate_memory(1024)?;
    let process = create_process()?;
    
    process.map_memory(memory)?;
    
    Ok(Data::new(process, memory))
}

/// Provide context with map_err
pub fn read_file(path: &Path) -> Result<Vec<u8>, FileError> {
    std::fs::read(path)
        .map_err(|e| FileError::ReadFailed {
            path: path.to_owned(),
            source: e,
        })
}

/// Use anyhow for application-level errors
pub fn main_application() -> anyhow::Result<()> {
    let config = load_config()
        .context("Failed to load configuration")?;
    
    let server = Server::new(&config)
        .context("Failed to create server")?;
    
    server.run()
        .context("Server failed during execution")?;
    
    Ok(())
}
```

### Panic Guidelines

```rust
/// Panics should be documented
/// 
/// # Panics
/// 
/// Panics if `index` is out of bounds.
pub fn get_unchecked(&self, index: usize) -> &T {
    &self.data[index]
}

/// Prefer returning errors over panicking
pub fn get(&self, index: usize) -> Option<&T> {
    self.data.get(index)
}

/// Use debug_assert! for invariants
pub fn internal_operation(&mut self) {
    debug_assert!(!self.data.is_empty(), "Data must not be empty");
    debug_assert!(self.is_valid(), "Invalid state detected");
    
    // Proceed with operation
}
```

## Memory Management

### Ownership Patterns

```rust
/// Clear ownership transfer
pub struct UniqueResource {
    handle: NonNull<Resource>,
}

impl UniqueResource {
    /// Takes ownership of raw resource
    pub unsafe fn from_raw(ptr: *mut Resource) -> Self {
        Self {
            handle: NonNull::new_unchecked(ptr),
        }
    }
    
    /// Returns ownership to caller
    pub fn into_raw(self) -> *mut Resource {
        let ptr = self.handle.as_ptr();
        mem::forget(self);
        ptr
    }
}

impl Drop for UniqueResource {
    fn drop(&mut self) {
        unsafe {
            dealloc_resource(self.handle.as_ptr());
        }
    }
}

/// Shared ownership with reference counting
pub struct SharedResource {
    inner: Arc<ResourceInner>,
}

impl SharedResource {
    pub fn new(data: ResourceData) -> Self {
        Self {
            inner: Arc::new(ResourceInner { data }),
        }
    }
    
    pub fn clone_ref(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
```

### Lifetime Management

```rust
/// Explicit lifetime relationships
pub struct MemoryRegion<'a> {
    mapping: &'a MemoryMapping,
    range: Range<usize>,
}

impl<'a> MemoryRegion<'a> {
    /// Lifetime tied to parent mapping
    pub fn subregion(&self, range: Range<usize>) -> MemoryRegion<'_> {
        assert!(range.end <= self.range.len());
        MemoryRegion {
            mapping: self.mapping,
            range: self.range.start + range.start..self.range.start + range.end,
        }
    }
}

/// Self-referential structures using Pin
pub struct AsyncTask {
    future: Pin<Box<dyn Future<Output = ()> + Send>>,
    waker: Option<Waker>,
}

impl AsyncTask {
    pub fn new<F>(future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self {
            future: Box::pin(future),
            waker: None,
        }
    }
}
```

### Zero-Copy APIs

```rust
/// Provide zero-copy access when possible
pub struct Buffer {
    data: Vec<u8>,
}

impl Buffer {
    /// Zero-copy read
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }
    
    /// Zero-copy write access
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
    
    /// Copy-on-write for sharing
    pub fn cow_slice(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.data)
    }
    
    /// Avoid unnecessary allocations
    pub fn extend_from_slice(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data);
    }
}
```

## Concurrency Patterns

### Synchronization Primitives

```rust
/// Interior mutability with clear semantics
pub struct ThreadSafeCounter {
    value: AtomicU64,
}

impl ThreadSafeCounter {
    pub fn new(initial: u64) -> Self {
        Self {
            value: AtomicU64::new(initial),
        }
    }
    
    /// Non-blocking increment
    pub fn increment(&self) -> u64 {
        self.value.fetch_add(1, Ordering::Relaxed)
    }
    
    /// Current value (may be stale)
    pub fn load(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Explicit locking with RAII guards
pub struct SharedState<T> {
    inner: Mutex<T>,
}

impl<T> SharedState<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }
    
    /// Blocking lock acquisition
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.inner.lock().unwrap()
    }
    
    /// Non-blocking attempt
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock().ok()
    }
}
```

### Async Patterns

```rust
/// Async-first APIs where appropriate
pub trait AsyncFileSystem {
    /// Async file operations
    async fn read(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write(&self, path: &Path, data: &[u8]) -> Result<()>;
    async fn delete(&self, path: &Path) -> Result<()>;
}

/// Cancellation-safe async operations
pub struct AsyncOperation {
    state: Arc<Mutex<OperationState>>,
    cancel_token: CancellationToken,
}

impl AsyncOperation {
    pub async fn run(&self) -> Result<()> {
        tokio::select! {
            result = self.do_work() => result,
            _ = self.cancel_token.cancelled() => {
                self.cleanup().await;
                Err(Error::Cancelled)
            }
        }
    }
    
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}
```

### Send and Sync

```rust
/// Document Send/Sync requirements
pub struct WorkQueue<T: Send> {
    queue: SegQueue<T>,
}

/// Explicitly not Send/Sync
pub struct LocalHandle {
    id: usize,
    _not_send: PhantomData<*const ()>,
}

/// Conditional Send/Sync
pub struct Container<T> {
    data: T,
}

// Container is Send if T is Send
unsafe impl<T: Send> Send for Container<T> {}

// Container is Sync if T is Sync
unsafe impl<T: Sync> Sync for Container<T> {}
```

## API Stability

### Versioning

```rust
/// Use semver-compatible versioning
#[derive(Debug)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const CURRENT: Self = Self {
        major: 1,
        minor: 0,
        patch: 0,
    };
    
    pub fn is_compatible(&self, other: &Self) -> bool {
        self.major == other.major && self.minor >= other.minor
    }
}
```

### Deprecation

```rust
/// Deprecate with clear migration path
#[deprecated(
    since = "0.5.0",
    note = "Use `allocate_frames` instead for better performance"
)]
pub fn allocate_frame() -> Option<Frame> {
    allocate_frames(1).ok().map(|mut frames| frames.pop().unwrap())
}

/// Feature flags for experimental APIs
#[cfg(feature = "experimental")]
pub mod experimental {
    /// Unstable API - may change without notice
    pub fn new_feature() -> Result<()> {
        todo!("Experimental feature")
    }
}
```

### Breaking Changes

```rust
/// Use non_exhaustive for future compatibility
#[non_exhaustive]
pub enum SystemCall {
    Read,
    Write,
    Open,
    Close,
    // Future variants can be added
}

#[non_exhaustive]
pub struct Config {
    pub timeout: Duration,
    pub retry_count: u32,
    // Future fields can be added
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            retry_count: 3,
        }
    }
}
```

## Documentation Standards

### Module Documentation

```rust
//! # Memory Management Module
//! 
//! This module provides memory allocation and management primitives
//! for the Veridian OS kernel.
//! 
//! ## Overview
//! 
//! The memory manager uses a buddy allocator for physical frames
//! and a slab allocator for kernel objects.
//! 
//! ## Examples
//! 
//! ```
//! use veridian::memory::{FrameAllocator, Frame};
//! 
//! let mut allocator = FrameAllocator::new();
//! let frame = allocator.allocate().expect("out of memory");
//! 
//! // Use frame...
//! 
//! allocator.deallocate(frame);
//! ```
//! 
//! ## Performance
//! 
//! - Frame allocation: O(log n) where n is number of free lists
//! - Frame deallocation: O(log n) due to buddy merging
//! - Memory overhead: ~0.1% for metadata
```

### Function Documentation

```rust
/// Allocates a contiguous region of physical memory.
/// 
/// This function attempts to allocate `count` contiguous frames
/// from the physical memory allocator. It uses a best-fit strategy
/// to minimize fragmentation.
/// 
/// # Arguments
/// 
/// * `count` - Number of contiguous frames to allocate
/// * `align` - Alignment requirement in bytes (must be power of 2)
/// 
/// # Returns
/// 
/// Returns a vector of allocated frames on success, or an error
/// if the allocation cannot be satisfied.
/// 
/// # Errors
/// 
/// - [`MemoryError::OutOfMemory`] - Not enough contiguous frames
/// - [`MemoryError::InvalidAlignment`] - Alignment is not power of 2
/// 
/// # Examples
/// 
/// ```
/// # use veridian::memory::*;
/// # let mut allocator = FrameAllocator::new();
/// // Allocate 16 contiguous frames (64 KiB)
/// let frames = allocator.allocate_contiguous(16, 4096)?;
/// assert_eq!(frames.len(), 16);
/// 
/// // Frames are contiguous
/// for i in 1..frames.len() {
///     assert_eq!(
///         frames[i].start_address(),
///         frames[i-1].start_address() + 4096
///     );
/// }
/// # Ok::<(), MemoryError>(())
/// ```
/// 
/// # Performance
/// 
/// Time complexity: O(n) where n is the number of free regions
/// Space complexity: O(1)
/// 
/// # Safety
/// 
/// This function is safe to call from multiple threads concurrently.
/// Internal locking ensures thread safety.
pub fn allocate_contiguous(
    &mut self,
    count: usize,
    align: usize,
) -> Result<Vec<Frame>> {
    // Implementation...
}
```

## Examples

### Complete API Example

```rust
//! # Process Management API
//! 
//! Complete example of a well-designed API following guidelines.

use std::sync::Arc;
use std::time::Duration;

/// Process identifier - guaranteed unique
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(u64);

/// Process priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Priority {
    Idle = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Realtime = 4,
}

/// Process creation configuration
#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub name: String,
    pub priority: Priority,
    pub memory_limit: Option<usize>,
    pub cpu_affinity: Option<CpuSet>,
    pub capabilities: CapabilitySet,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            name: String::from("unnamed"),
            priority: Priority::Normal,
            memory_limit: None,
            cpu_affinity: None,
            capabilities: CapabilitySet::default(),
        }
    }
}

/// Process handle for user interaction
#[derive(Clone)]
pub struct Process {
    inner: Arc<ProcessInner>,
}

impl Process {
    /// Creates a new process with the given configuration.
    /// 
    /// # Errors
    /// 
    /// Returns an error if:
    /// - The system is out of process IDs
    /// - Memory limit exceeds system capacity
    /// - Required capabilities cannot be granted
    pub fn new(config: ProcessConfig) -> Result<Self> {
        let inner = ProcessInner::create(config)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }
    
    /// Returns the process ID.
    pub fn id(&self) -> ProcessId {
        self.inner.id
    }
    
    /// Returns the current process state.
    pub fn state(&self) -> ProcessState {
        self.inner.state.load()
    }
    
    /// Waits for the process to terminate.
    /// 
    /// # Returns
    /// 
    /// Returns the exit code when the process terminates.
    pub async fn wait(&self) -> i32 {
        self.inner.wait_handle.wait().await
    }
    
    /// Terminates the process.
    /// 
    /// # Errors
    /// 
    /// Returns an error if the process has already terminated
    /// or if the caller lacks permission.
    pub fn terminate(&self) -> Result<()> {
        self.inner.terminate()
    }
}

// Internal implementation details
struct ProcessInner {
    id: ProcessId,
    state: AtomicState,
    wait_handle: WaitHandle,
    // ...
}
```

## Anti-Patterns

### What to Avoid

```rust
// ❌ Bad: Unclear naming
pub fn proc_mgr_init() { }

// ✅ Good: Clear, descriptive naming
pub fn initialize_process_manager() { }

// ❌ Bad: Using unwrap in library code
pub fn get_process(id: ProcessId) -> Process {
    PROCESS_TABLE.lock().unwrap().get(id).unwrap().clone()
}

// ✅ Good: Proper error handling
pub fn get_process(id: ProcessId) -> Result<Process> {
    PROCESS_TABLE
        .lock()
        .map_err(|_| Error::LockPoisoned)?
        .get(id)
        .cloned()
        .ok_or(Error::ProcessNotFound(id))
}

// ❌ Bad: Leaking implementation details
pub struct Buffer {
    pub vec: Vec<u8>,  // Don't expose internals
    pub pos: usize,
}

// ✅ Good: Encapsulation
pub struct Buffer {
    data: Vec<u8>,
    position: usize,
}

impl Buffer {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Controlled access through methods
    }
}

// ❌ Bad: Stringly-typed APIs
pub fn set_config(key: &str, value: &str) { }

// ✅ Good: Type-safe configuration
pub enum ConfigKey {
    MaxProcesses,
    DefaultPriority,
    MemoryLimit,
}

pub enum ConfigValue {
    Number(u64),
    Priority(Priority),
    Size(usize),
}

pub fn set_config(key: ConfigKey, value: ConfigValue) { }
```

## Conclusion

Well-designed APIs are crucial for Veridian OS’s success. By following these guidelines:

1. **Make APIs hard to misuse** through type safety and clear contracts
1. **Be consistent** across the entire codebase
1. **Document thoroughly** including examples and edge cases
1. **Design for evolution** with non-exhaustive enums and versioning
1. **Prioritize safety and performance** equally

Remember: APIs are forever. Take time to design them well, as changing them later is costly for users. When in doubt, start with a minimal API—it’s easier to add features than remove them.