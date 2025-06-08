# API Design Guide

This guide defines the principles and patterns for designing APIs in VeridianOS. Following these guidelines ensures consistency, type safety, and optimal performance across the system.

## Core Principles

### 1. Type Safety First
Use Rust's type system to prevent errors at compile time:

```rust
// Bad: Using raw integers
fn set_priority(priority: i32) { }

// Good: Type-safe enums
pub enum Priority {
    Idle = 0,
    Normal = 1,
    High = 2,
    RealTime = 3,
}

fn set_priority(priority: Priority) { }
```

### 2. Zero-Cost Abstractions
Abstractions should have no runtime overhead:

```rust
// Newtype pattern for type safety without cost
#[repr(transparent)]
pub struct ProcessId(u32);

#[repr(transparent)]
pub struct ThreadId(u32);

// Different types prevent mixing IDs
fn kill_process(pid: ProcessId) { }
fn kill_thread(tid: ThreadId) { }
```

### 3. Explicit Error Handling
All fallible operations return `Result`:

```rust
// Bad: Panic on error
pub fn allocate_memory(size: usize) -> *mut u8 {
    panic!("Out of memory")
}

// Good: Explicit error handling
pub fn allocate_memory(size: usize) -> Result<*mut u8, AllocError> {
    Err(AllocError::OutOfMemory)
}
```

## Naming Conventions

### Types and Traits
```rust
// Types: UpperCamelCase
pub struct MemoryRegion { }
pub enum PageFault { }
pub trait Scheduler { }

// Associated types
type Item = ProcessId;
type Error = SchedulerError;
```

### Functions and Methods
```rust
// Functions: snake_case
pub fn create_process() -> Process { }
pub fn map_memory() -> Result<VirtAddr, Error> { }

// Predicates: is_ or has_ prefix
pub fn is_ready(&self) -> bool { }
pub fn has_capability(&self, cap: &Capability) -> bool { }

// Conversions: from_, to_, into_, as_
pub fn from_bytes(bytes: &[u8]) -> Self { }
pub fn to_string(&self) -> String { }
pub fn into_inner(self) -> T { }
pub fn as_ptr(&self) -> *const T { }
```

### Constants and Statics
```rust
// Constants: SCREAMING_SNAKE_CASE
pub const PAGE_SIZE: usize = 4096;
pub const MAX_PROCESSES: usize = 1024;

// Statics: SCREAMING_SNAKE_CASE
pub static KERNEL_VERSION: &str = "0.1.0";
```

## API Patterns

### Builder Pattern
For complex object construction:

```rust
pub struct ProcessBuilder {
    name: Option<String>,
    priority: Priority,
    capabilities: Vec<Capability>,
}

impl ProcessBuilder {
    pub fn new() -> Self {
        Self {
            name: None,
            priority: Priority::Normal,
            capabilities: Vec::new(),
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
    
    pub fn capability(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }
    
    pub fn build(self) -> Result<Process, ProcessError> {
        // Validation and construction
        Ok(Process { /* ... */ })
    }
}

// Usage
let process = ProcessBuilder::new()
    .name("init")
    .priority(Priority::High)
    .capability(root_cap)
    .build()?;
```

### Type State Pattern
Encode state in the type system:

```rust
pub struct Port<S> {
    id: PortId,
    _state: PhantomData<S>,
}

pub struct Unbound;
pub struct Bound;
pub struct Connected;

impl Port<Unbound> {
    pub fn bind(self, addr: Address) -> Result<Port<Bound>, Error> {
        // Binding logic
        Ok(Port { id: self.id, _state: PhantomData })
    }
}

impl Port<Bound> {
    pub fn listen(self) -> Result<Port<Connected>, Error> {
        // Listen logic
        Ok(Port { id: self.id, _state: PhantomData })
    }
}

// Can only send on connected ports
impl Port<Connected> {
    pub fn send(&self, msg: Message) -> Result<(), Error> {
        // Send logic
    }
}
```

### Extension Traits
Add methods to existing types:

```rust
pub trait ProcessExt {
    fn spawn_thread(&self) -> Result<Thread, Error>;
    fn nice(&mut self, adjustment: i32) -> Result<(), Error>;
}

impl ProcessExt for Process {
    fn spawn_thread(&self) -> Result<Thread, Error> {
        // Implementation
    }
    
    fn nice(&mut self, adjustment: i32) -> Result<(), Error> {
        // Implementation
    }
}
```

## Memory Management APIs

### Ownership Transfer
Make ownership explicit:

```rust
// Take ownership
pub fn consume_buffer(buffer: Buffer) {
    // Buffer is moved, caller can't use it
}

// Borrow
pub fn read_buffer(buffer: &Buffer) -> &[u8] {
    // Buffer is borrowed, caller retains ownership
}

// Mutable borrow
pub fn modify_buffer(buffer: &mut Buffer) {
    // Buffer is mutably borrowed
}
```

### Lifetime Management
Express relationships between references:

```rust
pub struct MemoryMapping<'a> {
    region: &'a MemoryRegion,
    base: VirtAddr,
}

impl<'a> MemoryMapping<'a> {
    pub fn new(region: &'a MemoryRegion) -> Self {
        // Mapping lifetime tied to region
        Self { region, base: map(region) }
    }
}
```

## Capability APIs

### Capability Operations
Type-safe capability handling:

```rust
pub trait CapabilityHolder {
    fn capability(&self) -> &Capability;
    
    fn check_rights(&self, required: Rights) -> Result<(), CapError> {
        if self.capability().rights.contains(required) {
            Ok(())
        } else {
            Err(CapError::InsufficientRights)
        }
    }
}

// Type-safe wrappers
pub struct FileCapability(Capability);
pub struct ProcessCapability(Capability);

impl FileCapability {
    pub fn read(&self) -> Result<Vec<u8>, Error> {
        self.check_rights(Rights::READ)?;
        // Read implementation
    }
}
```

## Async APIs

### Future-based APIs
For asynchronous operations:

```rust
pub trait AsyncPort {
    async fn send(&self, msg: Message) -> Result<(), Error>;
    async fn receive(&self) -> Result<Message, Error>;
}

// Timeout support
pub async fn receive_timeout(
    port: &impl AsyncPort,
    timeout: Duration,
) -> Result<Message, Error> {
    tokio::time::timeout(timeout, port.receive())
        .await
        .map_err(|_| Error::Timeout)?
}
```

## Performance Guidelines

### Inline Hints
Use for hot paths:

```rust
#[inline]
pub fn page_number(addr: VirtAddr) -> usize {
    addr.0 / PAGE_SIZE
}

#[inline(always)]
pub fn is_aligned(addr: VirtAddr) -> bool {
    addr.0 & (PAGE_SIZE - 1) == 0
}
```

### Const Functions
Enable compile-time evaluation:

```rust
pub const fn kb(n: usize) -> usize {
    n * 1024
}

pub const fn mb(n: usize) -> usize {
    n * 1024 * 1024
}

// Usage in constants
const KERNEL_HEAP_SIZE: usize = mb(16);
```

## Documentation Standards

### Module Documentation
```rust
//! # Process Management
//!
//! This module provides process creation, scheduling, and lifecycle
//! management for VeridianOS.
//!
//! ## Examples
//!
//! ```no_run
//! let process = Process::create("init")?;
//! process.start()?;
//! ```
```

### Function Documentation
```rust
/// Creates a new process with the given name and capabilities.
///
/// # Arguments
///
/// * `name` - Human-readable process name
/// * `capabilities` - Initial capability set
///
/// # Returns
///
/// Returns the created process or an error if creation fails.
///
/// # Errors
///
/// * `ProcessError::TooManyProcesses` - Process table full
/// * `ProcessError::InvalidName` - Name contains invalid characters
///
/// # Examples
///
/// ```
/// let process = create_process("worker", &[memory_cap, file_cap])?;
/// ```
pub fn create_process(
    name: &str,
    capabilities: &[Capability],
) -> Result<Process, ProcessError> {
    // Implementation
}
```

## Versioning

### API Stability
Mark stability levels:

```rust
#[stable(feature = "process_api", since = "0.1.0")]
pub fn create_process() -> Process { }

#[unstable(feature = "async_ipc", issue = "42")]
pub async fn async_send() -> Result<(), Error> { }

#[deprecated(since = "0.2.0", note = "Use create_process instead")]
pub fn spawn_process() -> Process { }
```

## Best Practices

1. **Prefer static dispatch**: Use generics over trait objects
2. **Minimize allocations**: Use stack allocation where possible
3. **Make invalid states unrepresentable**: Use the type system
4. **Fail fast**: Validate inputs early
5. **Document invariants**: Especially for unsafe code
6. **Benchmark APIs**: Ensure performance meets targets