// Scheduler module

#[allow(dead_code)]
pub fn init() {
    println!("[SCHED] Initializing scheduler...");
    // TODO: Initialize task structures
    // TODO: Create idle task
    // TODO: Set up scheduling timer
    println!("[SCHED] Scheduler initialized");
}

#[allow(dead_code)]
pub fn run() -> ! {
    println!("[SCHED] Entering scheduler main loop");
    loop {
        // TODO: Schedule next task
        // TODO: Context switch
        crate::arch::idle();
    }
}
