# Phase 1: Microkernel Core Implementation (Months 4-9)

## Overview

Phase 1 implements the core microkernel functionality, establishing the foundation for all system services. This phase focuses on memory management, process scheduling, inter-process communication (IPC), and the capability system that forms the security backbone of VeridianOS.

## Objectives

1. **Memory Management**: Physical and virtual memory management with NUMA support
2. **Process Management**: Process creation, scheduling, and lifecycle management
3. **Inter-Process Communication**: High-performance, secure IPC mechanisms
4. **Capability System**: Unforgeable tokens for resource access control
5. **Interrupt Handling**: Efficient interrupt routing and handling
6. **System Call Interface**: Minimal, secure system call API

## Architecture Components

### 1. Memory Management Subsystem

#### 1.1 Physical Memory Manager

**kernel/src/mm/physical/mod.rs**
```rust
use core::mem::size_of;
use spin::Mutex;
use crate::arch::{PhysAddr, PAGE_SIZE};

/// Physical frame allocator using hybrid approach
pub struct FrameAllocator {
    /// Buddy allocator for large allocations
    buddy: BuddyAllocator,
    /// Bitmap allocator for single frames
    bitmap: BitmapAllocator,
    /// NUMA node information
    numa_nodes: Vec<NumaNode>,
    /// Statistics
    stats: FrameStats,
}

impl FrameAllocator {
    /// Initialize from memory map
    pub fn new(memory_map: &[MemoryRegion]) -> Self {
        let mut allocator = Self {
            buddy: BuddyAllocator::new(),
            bitmap: BitmapAllocator::new(),
            numa_nodes: Vec::new(),
            stats: FrameStats::default(),
        };
        
        // Initialize from memory regions
        for region in memory_map {
            if region.is_usable() {
                allocator.add_region(region.start, region.end);
            }
        }
        
        allocator
    }
    
    /// Allocate a single frame
    pub fn allocate_frame(&mut self) -> Option<PhysAddr> {
        // Try bitmap first for single frames
        if let Some(frame) = self.bitmap.allocate() {
            self.stats.allocated += 1;
            return Some(frame);
        }
        
        // Fall back to buddy allocator
        self.buddy.allocate_order(0)
    }
    
    /// Allocate contiguous frames
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<PhysAddr> {
        let order = count.next_power_of_two().trailing_zeros() as usize;
        self.buddy.allocate_order(order)
    }
    
    /// NUMA-aware allocation
    pub fn allocate_on_node(&mut self, node: usize) -> Option<PhysAddr> {
        if let Some(numa_node) = self.numa_nodes.get_mut(node) {
            numa_node.allocate_frame()
        } else {
            self.allocate_frame()
        }
    }
}

/// NUMA node representation
struct NumaNode {
    id: usize,
    memory_start: PhysAddr,
    memory_end: PhysAddr,
    local_allocator: BitmapAllocator,
    distance_map: Vec<u8>,
}

/// Memory region from firmware
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: PhysAddr,
    pub end: PhysAddr,
    pub typ: MemoryType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemoryType {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    Persistent,
}
```

#### 1.2 Virtual Memory Manager

**kernel/src/mm/virtual/mod.rs**
```rust
use crate::arch::{VirtAddr, PhysAddr, PageTable};
use crate::mm::physical::FrameAllocator;
use bitflags::bitflags;

bitflags! {
    /// Page table entry flags
    pub struct PageFlags: u64 {
        const PRESENT    = 1 << 0;
        const WRITABLE   = 1 << 1;
        const USER       = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const NO_CACHE   = 1 << 4;
        const ACCESSED   = 1 << 5;
        const DIRTY      = 1 << 6;
        const HUGE       = 1 << 7;
        const GLOBAL     = 1 << 8;
        const NO_EXECUTE = 1 << 63;
    }
}

/// Address space for a process
pub struct AddressSpace {
    /// Root page table
    root_table: PhysAddr,
    /// Memory mappings
    mappings: BTreeMap<VirtAddr, Mapping>,
    /// Next available address for mmap
    mmap_base: VirtAddr,
}

impl AddressSpace {
    /// Create new address space
    pub fn new(frame_allocator: &mut FrameAllocator) -> Result<Self, Error> {
        // Allocate root page table
        let root_table = frame_allocator
            .allocate_frame()
            .ok_or(Error::OutOfMemory)?;
            
        // Initialize with kernel mappings
        let mut space = Self {
            root_table,
            mappings: BTreeMap::new(),
            mmap_base: VirtAddr::new(0x1000_0000_0000), // User space start
        };
        
        // Map kernel space
        space.map_kernel_space(frame_allocator)?;
        
        Ok(space)
    }
    
    /// Map a virtual address to physical
    pub fn map(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frame_allocator: &mut FrameAllocator,
    ) -> Result<(), Error> {
        let mut page_table = unsafe { 
            PageTable::from_phys(self.root_table) 
        };
        
        // Walk page table, creating entries as needed
        for level in (1..=4).rev() {
            let entry = page_table.entry_for_addr(virt, level);
            
            if level == 1 {
                // Leaf entry
                entry.set(phys, flags);
                break;
            } else if !entry.is_present() {
                // Allocate intermediate table
                let table_frame = frame_allocator
                    .allocate_frame()
                    .ok_or(Error::OutOfMemory)?;
                entry.set(table_frame, PageFlags::PRESENT | PageFlags::WRITABLE);
            }
            
            page_table = unsafe { 
                PageTable::from_phys(entry.phys_addr()) 
            };
        }
        
        // Record mapping
        self.mappings.insert(virt, Mapping { phys, flags, size: PAGE_SIZE });
        
        Ok(())
    }
    
    /// Memory map files or anonymous memory
    pub fn mmap(
        &mut self,
        size: usize,
        prot: Protection,
        flags: MapFlags,
        frame_allocator: &mut FrameAllocator,
    ) -> Result<VirtAddr, Error> {
        // Align size to page boundary
        let size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        
        // Find free virtual address range
        let virt_addr = self.find_free_range(size)?;
        
        // Map pages
        for offset in (0..size).step_by(PAGE_SIZE) {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or(Error::OutOfMemory)?;
                
            self.map(
                virt_addr + offset,
                frame,
                prot.into(),
                frame_allocator,
            )?;
        }
        
        Ok(virt_addr)
    }
}

/// Memory mapping
#[derive(Debug)]
struct Mapping {
    phys: PhysAddr,
    flags: PageFlags,
    size: usize,
}

/// Memory protection flags
pub struct Protection {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl From<Protection> for PageFlags {
    fn from(prot: Protection) -> Self {
        let mut flags = PageFlags::PRESENT | PageFlags::USER;
        if prot.write { flags |= PageFlags::WRITABLE; }
        if !prot.execute { flags |= PageFlags::NO_EXECUTE; }
        flags
    }
}
```

### 2. Process Management

#### 2.1 Process Structure

**kernel/src/process/mod.rs**
```rust
use alloc::{string::String, vec::Vec, sync::Arc};
use spin::RwLock;
use crate::mm::virtual::AddressSpace;
use crate::cap::CapabilitySpace;

/// Process ID type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pid(u32);

/// Process control block
pub struct Process {
    /// Unique process ID
    pub pid: Pid,
    /// Parent process ID
    pub parent: Option<Pid>,
    /// Process name
    pub name: String,
    /// Memory address space
    pub address_space: Arc<RwLock<AddressSpace>>,
    /// Capability space
    pub capabilities: Arc<RwLock<CapabilitySpace>>,
    /// Thread list
    pub threads: Vec<ThreadId>,
    /// Process state
    pub state: ProcessState,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Statistics
    pub stats: ProcessStats,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessState {
    Created,
    Ready,
    Running,
    Blocked(BlockReason),
    Zombie,
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockReason {
    WaitingForIpc,
    WaitingForIo,
    WaitingForChild,
    Sleeping(u64), // Wake time
}

/// Thread control block
pub struct Thread {
    /// Thread ID
    pub tid: ThreadId,
    /// Owning process
    pub process: Pid,
    /// CPU context
    pub context: Context,
    /// Kernel stack
    pub kernel_stack: VirtAddr,
    /// User stack pointer
    pub user_stack: VirtAddr,
    /// Thread state
    pub state: ThreadState,
    /// Priority
    pub priority: Priority,
    /// Time slice remaining
    pub time_slice: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

/// CPU context saved on context switch
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Context {
    // Architecture-specific registers
    #[cfg(target_arch = "x86_64")]
    pub regs: X86_64Registers,
    #[cfg(target_arch = "aarch64")]
    pub regs: AArch64Registers,
    #[cfg(target_arch = "riscv64")]
    pub regs: RiscV64Registers,
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct X86_64Registers {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64, pub rsp: u64,
    pub r8: u64,  pub r9: u64,  pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub rflags: u64,
    pub cs: u64,  pub ss: u64,
}

impl Process {
    /// Create new process
    pub fn new(
        name: String,
        parent: Option<Pid>,
        frame_allocator: &mut FrameAllocator,
    ) -> Result<Self, Error> {
        let pid = PID_ALLOCATOR.allocate();
        let address_space = AddressSpace::new(frame_allocator)?;
        let capabilities = CapabilitySpace::new();
        
        Ok(Self {
            pid,
            parent,
            name,
            address_space: Arc::new(RwLock::new(address_space)),
            capabilities: Arc::new(RwLock::new(capabilities)),
            threads: Vec::new(),
            state: ProcessState::Created,
            exit_code: None,
            stats: ProcessStats::default(),
        })
    }
    
    /// Create main thread
    pub fn create_main_thread(
        &mut self,
        entry_point: VirtAddr,
        stack_size: usize,
        frame_allocator: &mut FrameAllocator,
    ) -> Result<ThreadId, Error> {
        let tid = THREAD_ID_ALLOCATOR.allocate();
        
        // Allocate kernel stack
        let kernel_stack = self.address_space.write().mmap(
            KERNEL_STACK_SIZE,
            Protection { read: true, write: true, execute: false },
            MapFlags::PRIVATE,
            frame_allocator,
        )?;
        
        // Allocate user stack
        let user_stack = self.address_space.write().mmap(
            stack_size,
            Protection { read: true, write: true, execute: false },
            MapFlags::PRIVATE | MapFlags::USER,
            frame_allocator,
        )?;
        
        let thread = Thread {
            tid,
            process: self.pid,
            context: Context::new(entry_point, user_stack + stack_size),
            kernel_stack,
            user_stack,
            state: ThreadState::Ready,
            priority: Priority::Normal,
            time_slice: DEFAULT_TIME_SLICE,
        };
        
        THREAD_TABLE.insert(tid, thread);
        self.threads.push(tid);
        
        Ok(tid)
    }
}
```

#### 2.2 Scheduler

**kernel/src/sched/mod.rs**
```rust
use alloc::collections::{BTreeMap, VecDeque};
use spin::Mutex;
use crate::process::{Thread, ThreadId, ThreadState, Priority};

/// Multi-level feedback queue scheduler
pub struct Scheduler {
    /// Ready queues by priority
    ready_queues: [VecDeque<ThreadId>; Priority::COUNT],
    /// Currently running thread per CPU
    running: BTreeMap<CpuId, ThreadId>,
    /// Blocked threads
    blocked: BTreeMap<ThreadId, BlockInfo>,
    /// Load balancing info
    load_info: LoadInfo,
}

impl Scheduler {
    /// Create new scheduler
    pub const fn new() -> Self {
        Self {
            ready_queues: [VecDeque::new(); Priority::COUNT],
            running: BTreeMap::new(),
            blocked: BTreeMap::new(),
            load_info: LoadInfo::new(),
        }
    }
    
    /// Add thread to ready queue
    pub fn enqueue(&mut self, tid: ThreadId) {
        if let Some(thread) = THREAD_TABLE.get(&tid) {
            let priority = thread.priority as usize;
            self.ready_queues[priority].push_back(tid);
            self.load_info.total_ready += 1;
        }
    }
    
    /// Select next thread to run
    pub fn pick_next(&mut self, cpu: CpuId) -> Option<ThreadId> {
        // Try each priority level
        for queue in self.ready_queues.iter_mut() {
            if let Some(tid) = queue.pop_front() {
                self.running.insert(cpu, tid);
                self.load_info.total_ready -= 1;
                return Some(tid);
            }
        }
        
        None
    }
    
    /// Context switch to new thread
    pub fn switch_to(&mut self, from: ThreadId, to: ThreadId) {
        unsafe {
            // Save current context
            if let Some(from_thread) = THREAD_TABLE.get_mut(&from) {
                arch::save_context(&mut from_thread.context);
                from_thread.state = ThreadState::Ready;
            }
            
            // Load new context
            if let Some(to_thread) = THREAD_TABLE.get_mut(&to) {
                to_thread.state = ThreadState::Running;
                arch::load_context(&to_thread.context);
                arch::switch_address_space(to_thread.process);
            }
        }
    }
    
    /// Timer tick handler
    pub fn tick(&mut self, cpu: CpuId) {
        if let Some(tid) = self.running.get(&cpu).copied() {
            if let Some(thread) = THREAD_TABLE.get_mut(&tid) {
                thread.time_slice = thread.time_slice.saturating_sub(1);
                
                if thread.time_slice == 0 {
                    // Time slice expired, preempt
                    thread.time_slice = DEFAULT_TIME_SLICE;
                    self.preempt(cpu);
                }
            }
        }
    }
    
    /// Block thread
    pub fn block(&mut self, tid: ThreadId, reason: BlockReason) {
        if let Some(cpu) = self.find_cpu_running(tid) {
            self.running.remove(&cpu);
        }
        
        self.blocked.insert(tid, BlockInfo {
            reason,
            wake_time: match reason {
                BlockReason::Sleeping(time) => Some(time),
                _ => None,
            },
        });
        
        if let Some(thread) = THREAD_TABLE.get_mut(&tid) {
            thread.state = ThreadState::Blocked;
        }
    }
    
    /// Unblock thread
    pub fn unblock(&mut self, tid: ThreadId) {
        if self.blocked.remove(&tid).is_some() {
            self.enqueue(tid);
        }
    }
    
    /// Load balancing across CPUs
    pub fn balance_load(&mut self) {
        let cpu_count = CPU_COUNT.load(Ordering::Relaxed);
        if cpu_count <= 1 {
            return;
        }
        
        // Calculate load per CPU
        let mut cpu_loads = vec![0usize; cpu_count];
        for (cpu, _) in &self.running {
            cpu_loads[cpu.0 as usize] += 1;
        }
        
        // Find most and least loaded CPUs
        let (min_cpu, min_load) = cpu_loads.iter()
            .enumerate()
            .min_by_key(|(_, load)| *load)
            .unwrap();
        let (max_cpu, max_load) = cpu_loads.iter()
            .enumerate()
            .max_by_key(|(_, load)| *load)
            .unwrap();
        
        // Migrate threads if imbalanced
        if max_load - min_load > LOAD_BALANCE_THRESHOLD {
            // Migration logic here
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    Idle = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Realtime = 4,
}

impl Priority {
    const COUNT: usize = 5;
}

struct BlockInfo {
    reason: BlockReason,
    wake_time: Option<u64>,
}

struct LoadInfo {
    total_ready: usize,
    total_blocked: usize,
    migrations: u64,
}
```

### 3. Inter-Process Communication

#### 3.1 Message Passing

**kernel/src/ipc/message.rs**
```rust
use alloc::{vec::Vec, sync::Arc};
use spin::Mutex;
use crate::cap::{Capability, CapabilityId};
use crate::process::Pid;

/// IPC message
#[derive(Debug, Clone)]
pub struct Message {
    /// Message header
    pub header: MessageHeader,
    /// Message payload
    pub data: Vec<u8>,
    /// Capabilities being transferred
    pub capabilities: Vec<CapabilityId>,
}

#[derive(Debug, Clone, Copy)]
pub struct MessageHeader {
    /// Sender process
    pub sender: Pid,
    /// Message type/tag
    pub msg_type: u32,
    /// Message ID for request/response matching
    pub msg_id: u64,
    /// Timestamp
    pub timestamp: u64,
}

/// Synchronous IPC endpoint
pub struct Endpoint {
    /// Endpoint ID
    pub id: EndpointId,
    /// Owner process
    pub owner: Pid,
    /// Waiting senders
    pub send_queue: VecDeque<(Pid, Message)>,
    /// Waiting receivers
    pub recv_queue: VecDeque<Pid>,
    /// Maximum message size
    pub max_msg_size: usize,
}

impl Endpoint {
    /// Send message (blocking)
    pub fn send(&mut self, sender: Pid, msg: Message) -> Result<(), IpcError> {
        // Check message size
        if msg.data.len() > self.max_msg_size {
            return Err(IpcError::MessageTooLarge);
        }
        
        // Check if receiver is waiting
        if let Some(receiver) = self.recv_queue.pop_front() {
            // Direct handoff
            deliver_message(receiver, msg)?;
            unblock_thread(receiver);
        } else {
            // Queue message and block sender
            self.send_queue.push_back((sender, msg));
            block_thread(sender, BlockReason::WaitingForIpc);
        }
        
        Ok(())
    }
    
    /// Receive message (blocking)
    pub fn receive(&mut self, receiver: Pid) -> Result<Message, IpcError> {
        // Check if message is waiting
        if let Some((sender, msg)) = self.send_queue.pop_front() {
            // Unblock sender
            unblock_thread(sender);
            Ok(msg)
        } else {
            // Block until message arrives
            self.recv_queue.push_back(receiver);
            block_thread(receiver, BlockReason::WaitingForIpc);
            Err(IpcError::WouldBlock)
        }
    }
}

/// Asynchronous channel
pub struct Channel {
    /// Channel ID
    pub id: ChannelId,
    /// Message buffer
    pub buffer: Arc<Mutex<RingBuffer<Message>>>,
    /// Subscribers
    pub subscribers: Vec<Pid>,
    /// Channel capacity
    pub capacity: usize,
}

impl Channel {
    /// Send message (non-blocking)
    pub fn send_async(&self, msg: Message) -> Result<(), IpcError> {
        let mut buffer = self.buffer.lock();
        
        if buffer.is_full() {
            return Err(IpcError::ChannelFull);
        }
        
        buffer.push(msg);
        
        // Wake any waiting receivers
        for &pid in &self.subscribers {
            if is_blocked_on_channel(pid, self.id) {
                unblock_thread(pid);
            }
        }
        
        Ok(())
    }
    
    /// Receive message (non-blocking)
    pub fn receive_async(&self) -> Option<Message> {
        self.buffer.lock().pop()
    }
}

/// Zero-copy shared memory region
pub struct SharedMemory {
    /// Region ID
    pub id: SharedMemoryId,
    /// Physical pages backing the region
    pub pages: Vec<PhysAddr>,
    /// Size in bytes
    pub size: usize,
    /// Processes with access
    pub mappings: BTreeMap<Pid, SharedMapping>,
}

#[derive(Debug)]
struct SharedMapping {
    virt_addr: VirtAddr,
    permissions: Protection,
}

impl SharedMemory {
    /// Map into process address space
    pub fn map_into(
        &mut self,
        pid: Pid,
        permissions: Protection,
    ) -> Result<VirtAddr, IpcError> {
        let process = PROCESS_TABLE.get(&pid)
            .ok_or(IpcError::InvalidProcess)?;
            
        let mut addr_space = process.address_space.write();
        
        // Find free virtual address
        let virt_addr = addr_space.find_free_range(self.size)?;
        
        // Map each page
        for (i, &phys_addr) in self.pages.iter().enumerate() {
            let offset = i * PAGE_SIZE;
            addr_space.map(
                virt_addr + offset,
                phys_addr,
                permissions.into(),
                &mut *FRAME_ALLOCATOR.lock(),
            )?;
        }
        
        self.mappings.insert(pid, SharedMapping {
            virt_addr,
            permissions,
        });
        
        Ok(virt_addr)
    }
}
```

### 4. Capability System

#### 4.1 Capability Implementation

**kernel/src/cap/mod.rs**
```rust
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::collections::BTreeMap;
use spin::RwLock;

/// Capability ID - globally unique
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(u64);

/// Capability - unforgeable token for resource access
#[derive(Debug, Clone)]
pub struct Capability {
    /// Unique ID
    pub id: CapabilityId,
    /// Resource this capability grants access to
    pub resource: ResourceType,
    /// Permissions granted
    pub permissions: Permissions,
    /// Parent capability (for revocation)
    pub parent: Option<CapabilityId>,
    /// Whether this capability is revoked
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResourceType {
    /// Memory region
    Memory { start: PhysAddr, size: usize },
    /// IPC endpoint
    Endpoint(EndpointId),
    /// Process
    Process(Pid),
    /// Thread
    Thread(ThreadId),
    /// I/O port range (x86)
    IoPort { start: u16, end: u16 },
    /// Interrupt
    Interrupt(u8),
    /// Device
    Device(DeviceId),
    /// File
    File(FileId),
}

bitflags! {
    pub struct Permissions: u32 {
        const READ    = 1 << 0;
        const WRITE   = 1 << 1;
        const EXECUTE = 1 << 2;
        const GRANT   = 1 << 3;  // Can create derived capabilities
        const REVOKE  = 1 << 4;  // Can revoke derived capabilities
        const DESTROY = 1 << 5;  // Can destroy resource
    }
}

/// Per-process capability space
pub struct CapabilitySpace {
    /// Capabilities owned by this process
    capabilities: BTreeMap<CapabilityId, Capability>,
    /// Next slot for capability insertion
    next_slot: u32,
}

impl CapabilitySpace {
    pub fn new() -> Self {
        Self {
            capabilities: BTreeMap::new(),
            next_slot: 0,
        }
    }
    
    /// Insert capability
    pub fn insert(&mut self, cap: Capability) -> CapabilityId {
        let id = cap.id;
        self.capabilities.insert(id, cap);
        id
    }
    
    /// Lookup capability
    pub fn get(&self, id: CapabilityId) -> Option<&Capability> {
        self.capabilities.get(&id)
    }
    
    /// Check if capability grants permission
    pub fn check_permission(
        &self,
        cap_id: CapabilityId,
        required: Permissions,
    ) -> Result<(), CapError> {
        let cap = self.get(cap_id)
            .ok_or(CapError::InvalidCapability)?;
            
        if cap.revoked {
            return Err(CapError::RevokedCapability);
        }
        
        if !cap.permissions.contains(required) {
            return Err(CapError::InsufficientPermissions);
        }
        
        // Check parent capability chain
        if let Some(parent_id) = cap.parent {
            if !is_capability_valid(parent_id) {
                return Err(CapError::RevokedCapability);
            }
        }
        
        Ok(())
    }
    
    /// Derive new capability with reduced permissions
    pub fn derive(
        &mut self,
        parent_id: CapabilityId,
        new_perms: Permissions,
    ) -> Result<CapabilityId, CapError> {
        let parent = self.get(parent_id)
            .ok_or(CapError::InvalidCapability)?;
            
        // Check grant permission
        if !parent.permissions.contains(Permissions::GRANT) {
            return Err(CapError::InsufficientPermissions);
        }
        
        // New permissions must be subset of parent
        if !parent.permissions.contains(new_perms) {
            return Err(CapError::InvalidPermissions);
        }
        
        let new_cap = Capability {
            id: generate_capability_id(),
            resource: parent.resource.clone(),
            permissions: new_perms,
            parent: Some(parent_id),
            revoked: false,
        };
        
        Ok(self.insert(new_cap))
    }
    
    /// Revoke capability and all derived capabilities
    pub fn revoke(&mut self, cap_id: CapabilityId) -> Result<(), CapError> {
        let cap = self.capabilities.get_mut(&cap_id)
            .ok_or(CapError::InvalidCapability)?;
            
        // Check revoke permission
        if !cap.permissions.contains(Permissions::REVOKE) {
            return Err(CapError::InsufficientPermissions);
        }
        
        // Mark as revoked
        cap.revoked = true;
        
        // Revoke all derived capabilities
        let to_revoke: Vec<_> = self.capabilities
            .iter()
            .filter(|(_, c)| c.parent == Some(cap_id))
            .map(|(id, _)| *id)
            .collect();
            
        for id in to_revoke {
            self.revoke(id)?;
        }
        
        Ok(())
    }
}

/// Global capability ID generator
static CAP_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_capability_id() -> CapabilityId {
    CapabilityId(CAP_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// System capability table for validation
static SYSTEM_CAP_TABLE: RwLock<BTreeMap<CapabilityId, SystemCapEntry>> = 
    RwLock::new(BTreeMap::new());

struct SystemCapEntry {
    owner: Pid,
    resource: ResourceType,
    revoked: bool,
}

fn is_capability_valid(id: CapabilityId) -> bool {
    SYSTEM_CAP_TABLE.read()
        .get(&id)
        .map(|entry| !entry.revoked)
        .unwrap_or(false)
}
```

### 5. System Call Interface

#### 5.1 System Call Definitions

**kernel/src/syscall/mod.rs**
```rust
use crate::cap::{CapabilityId, Permissions};
use crate::ipc::{Message, EndpointId, ChannelId};
use crate::process::{Pid, ThreadId};

/// System call numbers
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum Syscall {
    // Process management
    ProcessCreate = 0,
    ProcessDestroy = 1,
    ThreadCreate = 2,
    ThreadExit = 3,
    ThreadYield = 4,
    
    // Memory management
    MemMap = 10,
    MemUnmap = 11,
    MemProtect = 12,
    
    // Capability management
    CapCreate = 20,
    CapDerive = 21,
    CapRevoke = 22,
    CapDestroy = 23,
    
    // IPC
    EndpointCreate = 30,
    EndpointSend = 31,
    EndpointReceive = 32,
    ChannelCreate = 33,
    ChannelSend = 34,
    ChannelReceive = 35,
    SharedMemCreate = 36,
    SharedMemMap = 37,
    
    // Time
    TimeGet = 40,
    TimeSleep = 41,
    
    // Debug
    DebugPrint = 50,
}

/// System call handler
pub fn syscall_handler(
    syscall: Syscall,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> Result<usize, SyscallError> {
    // Get current thread
    let current_thread = current_thread_id();
    let current_process = thread_to_process(current_thread);
    
    match syscall {
        Syscall::ProcessCreate => {
            let name_ptr = arg1 as *const u8;
            let name_len = arg2;
            let entry_point = arg3;
            
            // Validate and copy name from user space
            let name = copy_string_from_user(name_ptr, name_len)?;
            
            // Create process
            let pid = create_process(name, Some(current_process))?;
            
            Ok(pid.0 as usize)
        }
        
        Syscall::MemMap => {
            let size = arg1;
            let prot = Protection::from_bits(arg2 as u32)?;
            let flags = MapFlags::from_bits(arg3 as u32)?;
            
            let addr = mmap(current_process, size, prot, flags)?;
            
            Ok(addr.as_usize())
        }
        
        Syscall::EndpointCreate => {
            let max_msg_size = arg1;
            
            let endpoint_cap = create_endpoint(current_process, max_msg_size)?;
            
            Ok(endpoint_cap.0 as usize)
        }
        
        Syscall::EndpointSend => {
            let endpoint_cap = CapabilityId(arg1 as u64);
            let msg_ptr = arg2 as *const u8;
            let msg_len = arg3;
            let caps_ptr = arg4 as *const CapabilityId;
            let caps_len = arg5;
            
            // Validate capability
            check_capability(current_process, endpoint_cap, Permissions::WRITE)?;
            
            // Copy message from user
            let msg_data = copy_bytes_from_user(msg_ptr, msg_len)?;
            let caps = copy_caps_from_user(caps_ptr, caps_len)?;
            
            let msg = Message {
                header: MessageHeader {
                    sender: current_process,
                    msg_type: 0,
                    msg_id: generate_msg_id(),
                    timestamp: get_timestamp(),
                },
                data: msg_data,
                capabilities: caps,
            };
            
            endpoint_send(endpoint_cap, msg)?;
            
            Ok(0)
        }
        
        Syscall::ThreadYield => {
            yield_cpu();
            Ok(0)
        }
        
        Syscall::DebugPrint => {
            let str_ptr = arg1 as *const u8;
            let str_len = arg2;
            
            let string = copy_string_from_user(str_ptr, str_len)?;
            println!("[{}] {}", current_process.0, string);
            
            Ok(0)
        }
        
        _ => Err(SyscallError::InvalidSyscall),
    }
}

/// System call error types
#[derive(Debug)]
pub enum SyscallError {
    InvalidSyscall,
    InvalidArgument,
    AccessDenied,
    OutOfMemory,
    InvalidCapability,
    WouldBlock,
    ProcessNotFound,
    ResourceExhausted,
}

/// Copy data from user space with validation
fn copy_bytes_from_user(ptr: *const u8, len: usize) -> Result<Vec<u8>, SyscallError> {
    // Validate user pointer
    if !is_user_pointer(ptr, len) {
        return Err(SyscallError::InvalidArgument);
    }
    
    // Safe copy
    let mut buffer = vec![0u8; len];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, buffer.as_mut_ptr(), len);
    }
    
    Ok(buffer)
}
```

### 6. Interrupt Handling

#### 6.1 Interrupt Management

**kernel/src/interrupt/mod.rs**
```rust
use crate::process::ThreadId;
use crate::cap::CapabilityId;

/// Interrupt descriptor table
pub struct InterruptTable {
    /// Interrupt handlers
    handlers: [Option<InterruptHandler>; 256],
    /// Statistics
    stats: [InterruptStats; 256],
}

/// Interrupt handler
enum InterruptHandler {
    /// Kernel handler
    Kernel(fn(&mut InterruptFrame)),
    /// User handler (forwarded to user space)
    User {
        thread: ThreadId,
        capability: CapabilityId,
    },
}

/// Interrupt frame passed to handlers
#[repr(C)]
pub struct InterruptFrame {
    // Architecture-specific
    #[cfg(target_arch = "x86_64")]
    pub regs: X86_64InterruptFrame,
}

impl InterruptTable {
    /// Register kernel interrupt handler
    pub fn register_kernel_handler(
        &mut self,
        vector: u8,
        handler: fn(&mut InterruptFrame),
    ) {
        self.handlers[vector as usize] = Some(InterruptHandler::Kernel(handler));
    }
    
    /// Register user interrupt handler
    pub fn register_user_handler(
        &mut self,
        vector: u8,
        thread: ThreadId,
        capability: CapabilityId,
    ) -> Result<(), Error> {
        // Verify capability grants interrupt access
        check_capability(capability, Permissions::READ)?;
        
        self.handlers[vector as usize] = Some(InterruptHandler::User {
            thread,
            capability,
        });
        
        Ok(())
    }
    
    /// Main interrupt handler
    pub fn handle_interrupt(&mut self, vector: u8, frame: &mut InterruptFrame) {
        self.stats[vector as usize].count += 1;
        
        match self.handlers[vector as usize] {
            Some(InterruptHandler::Kernel(handler)) => {
                handler(frame);
            }
            Some(InterruptHandler::User { thread, .. }) => {
                // Queue interrupt for user handler
                queue_user_interrupt(thread, vector);
            }
            None => {
                // Spurious interrupt
                self.stats[vector as usize].spurious += 1;
            }
        }
        
        // Send EOI
        arch::end_of_interrupt(vector);
    }
}

#[derive(Default)]
struct InterruptStats {
    count: u64,
    spurious: u64,
    handling_time: u64,
}
```

## Implementation Timeline

### Month 4: Memory Management
- Week 1-2: Physical memory allocator (buddy + bitmap)
- Week 3-4: Virtual memory manager and page tables

### Month 5: Process Management
- Week 1-2: Process and thread structures
- Week 3-4: Context switching and basic scheduling

### Month 6: Scheduler
- Week 1-2: Multi-level feedback queue implementation
- Week 3-4: Load balancing and CPU affinity

### Month 7: IPC Foundation
- Week 1-2: Synchronous message passing
- Week 3-4: Asynchronous channels

### Month 8: Capability System
- Week 1-2: Capability implementation
- Week 3-4: Integration with resources

### Month 9: System Calls & Polish
- Week 1-2: System call interface
- Week 3-4: Testing and optimization

## Testing Strategy

### Unit Tests
- Memory allocator stress tests
- Page table manipulation tests
- Scheduler fairness tests
- Capability validation tests

### Integration Tests
- Process creation and destruction
- IPC performance benchmarks
- Context switch latency
- System call overhead

### System Tests
- Multi-process workloads
- Memory pressure scenarios
- Interrupt latency measurement
- Security boundary validation

## Success Criteria

1. **Memory Management**: Efficient allocation with <1μs latency
2. **Process Management**: Support 1000+ processes
3. **Scheduling**: Fair scheduling with <10μs context switch
4. **IPC**: <1μs for small message passing
5. **Capabilities**: Secure, unforgeable access control
6. **System Calls**: Complete minimal API (~50 calls)

## Dependencies for Phase 2

- Working process creation and management
- Functional IPC mechanisms
- Capability-based security
- Stable system call interface
- Basic device driver framework