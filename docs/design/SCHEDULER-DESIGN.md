# VeridianOS Scheduler Design Document

**Version**: 1.0  
**Date**: 2025-06-07  
**Status**: Draft for Phase 1 Implementation

## Executive Summary

This document defines the scheduler architecture for VeridianOS, targeting < 10μs context switch time while supporting real-time, interactive, and batch workloads. The design emphasizes scalability, fairness, and energy efficiency.

## Design Goals

### Performance Targets
- **Context Switch Time**: < 10μs
- **Scheduling Decision**: < 1μs
- **Wake-up Latency**: < 5μs
- **Load Balancing**: < 100μs
- **Scalability**: 1000+ concurrent processes

### Design Principles
1. **Multi-class**: Support RT, interactive, and batch workloads
2. **Fair**: Prevent starvation, ensure progress
3. **Scalable**: Per-CPU run queues, minimal contention
4. **Energy-Aware**: Consider power states in decisions
5. **Predictable**: Bounded latencies for RT tasks

## Process Model

### Thread Control Block (TCB)
```rust
#[repr(C)]
pub struct ThreadControlBlock {
    // Identity
    tid: ThreadId,
    pid: ProcessId,
    
    // Scheduling
    state: ThreadState,
    priority: Priority,
    sched_class: SchedClass,
    cpu_affinity: CpuSet,
    
    // Accounting
    cpu_time: u64,
    last_run: Instant,
    time_slice: Duration,
    
    // Context
    kernel_stack: VirtAddr,
    user_stack: VirtAddr,
    saved_context: Context,
    
    // Capabilities
    cap_space: CapabilitySpace,
}

#[derive(Debug, Clone, Copy)]
pub enum ThreadState {
    Running,
    Ready,
    Blocked(BlockReason),
    Suspended,
    Terminated,
}

#[derive(Debug, Clone, Copy)]
pub enum SchedClass {
    RealTime,      // Highest priority, time-critical
    Interactive,   // Low latency, responsive
    Normal,        // Standard time-sharing
    Batch,         // Throughput-oriented
    Idle,          // Lowest priority
}
```

### CPU Context
```rust
#[repr(C)]
pub struct Context {
    // General purpose registers
    rax: u64, rbx: u64, rcx: u64, rdx: u64,
    rsi: u64, rdi: u64, rbp: u64, rsp: u64,
    r8: u64,  r9: u64,  r10: u64, r11: u64,
    r12: u64, r13: u64, r14: u64, r15: u64,
    
    // Control registers
    rip: u64,
    rflags: u64,
    cr3: u64,  // Page table base
    
    // Segment registers
    cs: u16, ds: u16, es: u16, fs: u16, gs: u16, ss: u16,
    
    // FPU/SSE state pointer
    fpu_state: Option<Box<FpuState>>,
}
```

## Scheduling Algorithm

### Multi-Level Feedback Queue
```rust
pub struct Scheduler {
    /// Per-CPU scheduler instances
    cpus: Vec<CpuScheduler>,
    /// Global load balancer
    balancer: LoadBalancer,
    /// System-wide policies
    policies: SchedPolicies,
}

pub struct CpuScheduler {
    /// Current running thread
    current: Option<ThreadId>,
    /// Ready queues by class
    rt_queue: RealTimeQueue,
    interactive_queue: InteractiveQueue,
    normal_queue: NormalQueue,
    batch_queue: BatchQueue,
    /// Idle thread
    idle_thread: ThreadId,
    /// Local run queue lock
    lock: SpinLock<()>,
}
```

### Real-Time Scheduling
```rust
pub struct RealTimeQueue {
    /// Priority-ordered queue (0 = highest)
    priorities: [VecDeque<ThreadId>; RT_PRIORITIES],
    /// Deadline-based queue for EDF
    deadline_queue: BinaryHeap<DeadlineTask>,
}

impl RealTimeQueue {
    pub fn pick_next(&mut self) -> Option<ThreadId> {
        // Check deadline tasks first
        if let Some(task) = self.deadline_queue.peek() {
            if task.deadline <= now() + SCHEDULE_LATENCY {
                return self.deadline_queue.pop().map(|t| t.tid);
            }
        }
        
        // Fixed priority scheduling
        for queue in &mut self.priorities {
            if let Some(tid) = queue.pop_front() {
                return Some(tid);
            }
        }
        
        None
    }
}
```

### Interactive Scheduling
```rust
pub struct InteractiveQueue {
    /// Run queue with dynamic priorities
    queue: BinaryHeap<InteractiveTask>,
    /// Interactivity scores
    scores: HashMap<ThreadId, f32>,
}

impl InteractiveQueue {
    pub fn calculate_score(&self, thread: &TCB) -> f32 {
        let sleep_ratio = thread.sleep_time as f32 / thread.cpu_time as f32;
        let response_factor = 1.0 / (thread.avg_response_time + 1.0);
        
        sleep_ratio * 0.7 + response_factor * 0.3
    }
}
```

### CFS-like Fair Scheduling
```rust
pub struct NormalQueue {
    /// Red-black tree ordered by virtual runtime
    rbtree: RBTree<VRuntime, ThreadId>,
    /// Minimum virtual runtime
    min_vruntime: u64,
}

impl NormalQueue {
    pub fn pick_next(&mut self) -> Option<ThreadId> {
        self.rbtree.min().map(|(_, tid)| *tid)
    }
    
    pub fn update_vruntime(&mut self, tid: ThreadId, runtime: Duration) {
        let thread = get_thread(tid);
        let weight = weight_for_nice(thread.nice);
        let vruntime_delta = runtime.as_nanos() as u64 * NICE_0_WEIGHT / weight;
        
        thread.vruntime += vruntime_delta;
        self.rbtree.update(thread.vruntime, tid);
    }
}
```

## Context Switching

### Fast Context Switch Path
```asm
; x86_64 context switch
switch_context:
    ; Save current context
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    
    ; Save stack pointer
    mov [rdi + Context.rsp], rsp
    
    ; Load new context
    mov rsp, [rsi + Context.rsp]
    
    ; Restore registers
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    
    ; Switch page tables if needed
    mov rax, [rsi + Context.cr3]
    mov rbx, cr3
    cmp rax, rbx
    je .skip_cr3
    mov cr3, rax
.skip_cr3:
    ret
```

### FPU Context Handling
```rust
impl Scheduler {
    fn handle_fpu_context(&mut self, prev: &TCB, next: &TCB) {
        // Lazy FPU switching
        if prev.used_fpu {
            unsafe {
                // Save FPU state
                asm!("fxsave64 [{}]", in(reg) &mut prev.fpu_state);
            }
        }
        
        if next.fpu_state.is_some() {
            unsafe {
                // Restore FPU state
                asm!("fxrstor64 [{}]", in(reg) &next.fpu_state);
            }
        } else {
            // Clear FPU for new process
            unsafe {
                asm!("fninit");
            }
        }
    }
}
```

## Load Balancing

### Per-CPU Load Tracking
```rust
pub struct CpuLoad {
    /// Exponentially weighted moving average
    load_avg: f32,
    /// Current run queue length
    nr_running: u32,
    /// CPU utilization (0-100)
    utilization: u8,
    /// Power state
    power_state: PowerState,
}

pub struct LoadBalancer {
    /// Balancing interval
    interval: Duration,
    /// Load threshold for migration
    imbalance_threshold: f32,
}

impl LoadBalancer {
    pub fn balance(&mut self, cpus: &mut [CpuScheduler]) {
        let loads: Vec<_> = cpus.iter().map(|cpu| cpu.calculate_load()).collect();
        
        // Find busiest and least loaded CPUs
        let (busiest, least) = self.find_imbalance(&loads);
        
        if loads[busiest].load_avg - loads[least].load_avg > self.imbalance_threshold {
            self.migrate_tasks(busiest, least, cpus);
        }
    }
}
```

### Task Migration
```rust
impl LoadBalancer {
    fn migrate_tasks(&mut self, from: usize, to: usize, cpus: &mut [CpuScheduler]) {
        let mut migrated = 0;
        
        // Select tasks for migration
        while migrated < MAX_MIGRATE_TASKS {
            if let Some(task) = cpus[from].select_migration_candidate() {
                // Check CPU affinity
                if task.cpu_affinity.is_set(to) {
                    cpus[from].dequeue(task.tid);
                    cpus[to].enqueue(task.tid);
                    migrated += 1;
                }
            } else {
                break;
            }
        }
    }
}
```

## Synchronization Primitives

### Priority Inheritance
```rust
pub struct PriorityInheritance {
    /// Threads waiting on locks with priorities
    wait_graph: HashMap<LockId, Vec<(ThreadId, Priority)>>,
}

impl PriorityInheritance {
    pub fn boost_priority(&mut self, owner: ThreadId, waiter: &TCB) {
        let owner_tcb = get_thread_mut(owner);
        
        if waiter.priority > owner_tcb.priority {
            owner_tcb.inherited_priority = Some(waiter.priority);
            // Re-enqueue with new priority
            reschedule_thread(owner);
        }
    }
}
```

### Futex Support
```rust
pub struct Futex {
    /// Wait queues per futex address
    wait_queues: HashMap<VirtAddr, VecDeque<ThreadId>>,
}

impl Futex {
    pub fn wait(&mut self, addr: VirtAddr, expected: u32, timeout: Option<Duration>) -> Result<(), FutexError> {
        // Atomic check and sleep
        let value = unsafe { *(addr.as_ptr::<AtomicU32>()) };
        
        if value.load(Ordering::Acquire) != expected {
            return Err(FutexError::ValueMismatch);
        }
        
        let current = current_thread();
        self.wait_queues.entry(addr).or_default().push_back(current.tid);
        
        // Block thread
        current.block_on_futex(addr, timeout);
        schedule();
        
        Ok(())
    }
}
```

## Power Management

### CPU Frequency Scaling
```rust
pub struct PowerManager {
    /// Performance profiles
    profiles: Vec<PowerProfile>,
    /// Current profile per CPU
    cpu_profiles: Vec<usize>,
}

pub struct PowerProfile {
    name: &'static str,
    /// Target CPU frequency
    frequency: u32,
    /// Voltage setting
    voltage: f32,
    /// Transition latency
    transition_latency: Duration,
}

impl Scheduler {
    fn select_cpu_frequency(&mut self, cpu: usize) {
        let load = self.cpus[cpu].calculate_load();
        
        match load.utilization {
            0..=20 => self.power_mgr.set_profile(cpu, "powersave"),
            21..=70 => self.power_mgr.set_profile(cpu, "balanced"),
            71..=100 => self.power_mgr.set_profile(cpu, "performance"),
            _ => unreachable!(),
        }
    }
}
```

### Core Parking
```rust
impl PowerManager {
    pub fn park_idle_cores(&mut self, scheduler: &Scheduler) {
        for (cpu, sched) in scheduler.cpus.iter().enumerate() {
            if sched.is_idle() && self.can_park_core(cpu) {
                self.park_core(cpu);
            }
        }
    }
}
```

## Real-Time Guarantees

### Admission Control
```rust
pub struct AdmissionControl {
    /// CPU bandwidth reserved for RT tasks
    rt_bandwidth: f32,
    /// Currently admitted RT tasks
    admitted_tasks: HashMap<ThreadId, RtParameters>,
}

pub struct RtParameters {
    /// Worst-case execution time
    wcet: Duration,
    /// Period or deadline
    period: Duration,
    /// CPU bandwidth (wcet/period)
    bandwidth: f32,
}

impl AdmissionControl {
    pub fn admit_task(&mut self, params: RtParameters) -> Result<(), AdmissionError> {
        let total_bandwidth = self.calculate_total_bandwidth() + params.bandwidth;
        
        if total_bandwidth > self.rt_bandwidth {
            return Err(AdmissionError::InsufficientBandwidth);
        }
        
        // Schedulability test
        if !self.is_schedulable_with(params) {
            return Err(AdmissionError::NotSchedulable);
        }
        
        Ok(())
    }
}
```

## Performance Monitoring

### Scheduler Statistics
```rust
pub struct SchedStats {
    /// Context switches per second
    context_switches: AtomicU64,
    /// Average scheduling latency
    avg_latency: AtomicU64,
    /// Load balancing migrations
    migrations: AtomicU64,
    /// CPU idle time
    idle_time: AtomicU64,
}

pub struct PerThreadStats {
    /// Total CPU time
    cpu_time: u64,
    /// Total wait time
    wait_time: u64,
    /// Number of voluntary context switches
    voluntary_switches: u64,
    /// Number of involuntary context switches
    involuntary_switches: u64,
}
```

## Testing Strategy

### Unit Tests
- Queue operations correctness
- Priority calculations
- Load balancing decisions

### Integration Tests
- Multi-core scheduling
- Priority inheritance
- Real-time guarantees

### Stress Tests
- 1000+ thread creation/destruction
- Context switch storms
- Load balancing under pressure

### Benchmarks
- Context switch latency
- Scheduling decision time
- Wake-up latency
- Fair CPU distribution

## Implementation Phases

### Phase 1 (Basic Scheduler)
1. Simple round-robin scheduler
2. Basic context switching
3. Single run queue
4. < 10μs context switch

### Phase 2 (Multi-core)
1. Per-CPU run queues
2. Basic load balancing
3. CPU affinity support
4. Migration infrastructure

### Phase 3 (Advanced Features)
1. Multiple scheduling classes
2. Real-time scheduling
3. Priority inheritance
4. Power management

## Future Enhancements

### Phase 5 (Performance)
- Gang scheduling
- NUMA-aware scheduling
- Hardware scheduling assist
- Predictive scheduling

### Phase 6 (Advanced)
- Container-aware scheduling
- GPU scheduling integration
- Heterogeneous computing
- Machine learning optimization

---

*This document will evolve based on implementation experience and performance analysis.*