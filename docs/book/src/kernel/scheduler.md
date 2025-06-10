# Scheduler

The VeridianOS scheduler is responsible for managing process and thread execution across multiple CPUs, providing fair CPU time allocation while meeting real-time constraints.

## Current Status

As of June 10, 2025, the scheduler implementation is approximately 25% complete:

- ✅ **Core Structure**: Round-robin algorithm implemented
- ✅ **Idle Task**: Created and managed for each CPU
- ✅ **Timer Setup**: 10ms tick configured for all architectures
- ✅ **Process Integration**: Thread to Task conversion working
- ✅ **SMP Basics**: Per-CPU data structures in place
- ✅ **CPU Affinity**: Basic support for thread pinning
- 🔲 **Priority Scheduling**: Not yet implemented
- 🔲 **CFS Algorithm**: Planned for future
- 🔲 **Real-time Classes**: Not yet implemented

## Architecture

### Task Structure

```rust
pub struct Task {
    pub pid: ProcessId,
    pub tid: ThreadId,
    pub name: String,
    pub state: ProcessState,
    pub priority: Priority,
    pub sched_class: SchedClass,
    pub sched_policy: SchedPolicy,
    pub cpu_affinity: CpuSet,
    pub context: TaskContext,
    // ... additional fields
}
```

### Scheduling Classes

1. **Real-Time**: Highest priority, time-critical tasks
2. **Interactive**: Low latency, responsive tasks
3. **Normal**: Standard time-sharing tasks
4. **Batch**: Throughput-oriented tasks
5. **Idle**: Lowest priority tasks

### Core Components

#### Ready Queue
Currently uses a single global ready queue with spinlock protection. Future versions will implement per-CPU run queues for better scalability.

#### Timer Interrupts
- **x86_64**: Uses Programmable Interval Timer (PIT)
- **AArch64**: Uses Generic Timer
- **RISC-V**: Uses SBI timer interface

All architectures configured for 10ms tick (100Hz).

#### Context Switching
Leverages architecture-specific context switching implementations from the process management subsystem:
- x86_64: ~1000 cycles overhead
- AArch64: ~800 cycles overhead  
- RISC-V: ~900 cycles overhead

## Usage

### Creating and Scheduling a Task

```rust
// Create a process first
let pid = process::lifecycle::create_process("my_process".to_string(), 0)?;

// Get the process and create a thread
if let Some(proc) = process::table::get_process_mut(pid) {
    let tid = process::create_thread(entry_point, arg1, arg2, arg3)?;
    
    // Schedule the thread
    if let Some(thread) = proc.get_thread(tid) {
        sched::schedule_thread(pid, tid, thread)?;
    }
}
```

### CPU Affinity

```rust
// Set thread affinity to CPUs 0 and 2
thread.cpu_affinity.store(0b101, Ordering::Relaxed);
```

### Yielding CPU

```rust
// Voluntarily yield CPU to other tasks
sched::yield_cpu();
```

## Implementation Details

### Round-Robin Algorithm

The current implementation uses a simple round-robin scheduler:

1. Each task gets a fixed time slice (10ms)
2. On timer interrupt, current task is moved to end of queue
3. Next task in queue is scheduled
4. If no ready tasks, idle task runs

### Load Balancing

Basic load balancing framework implemented:
- Monitors CPU load levels
- Detects significant imbalances (>20% difference)
- Framework for task migration (not yet fully implemented)

### SMP Support

- Per-CPU data structures initialized
- CPU topology detection (up to 8 CPUs)
- Basic NUMA awareness in task placement
- Lock-free operations where possible

## Performance Targets

| Metric | Target | Current Status |
|--------|--------|----------------|
| Context Switch | < 10μs | Pending measurement |
| Scheduling Decision | < 1μs | Pending measurement |
| Wake-up Latency | < 5μs | Pending measurement |
| Load Balancing | < 100μs | Basic framework only |

## Future Enhancements

### Phase 1 Completion
- Priority-based scheduling
- Per-CPU run queues
- Full task migration
- Performance measurements

### Phase 2 (Multi-core)
- Advanced load balancing
- NUMA optimization
- CPU hotplug support

### Phase 3 (Advanced)
- CFS implementation
- Real-time scheduling
- Priority inheritance
- Power management

## API Reference

### Core Functions

- `sched::init()` - Initialize scheduler subsystem
- `sched::run()` - Start scheduler main loop
- `sched::yield_cpu()` - Yield CPU to other tasks
- `sched::schedule_thread()` - Schedule a thread for execution
- `sched::set_algorithm()` - Change scheduling algorithm

### Timer Functions

- `sched::timer_tick()` - Handle timer interrupt
- `arch::timer::setup_timer()` - Configure timer hardware

## See Also

- [Process Management](../architecture/processes.md)
- [Memory Management](../architecture/memory.md)
- [Inter-Process Communication](../architecture/ipc.md)
