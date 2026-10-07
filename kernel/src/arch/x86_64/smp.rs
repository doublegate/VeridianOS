//! x86_64 secondary CPU bring-up (INIT-SIPI-SIPI), for `arch::smp_boot`.
//!
//! Application processors come from the ACPI MADT (enabled Local APIC
//! entries). Each starts in real mode at the trampoline page below 1 MiB,
//! which switches to protected mode and then to long mode on a temporary
//! page table, jumps to `veridian_ap_entry64` in kernel text, and from there
//! onto the kernel page table and its own stack. The sequence mirrors
//! Linux's `trampoline_64.S` (real mode -> `startup_32` -> `startup_64`).
//!
//! Stage S1 (ADR 0004): an AP loads its own GDT/TSS, the shared IDT, the
//! boot CPU's PAT, enables its Local APIC and tick, reports ONLINE and halts
//! with interrupts on. It never enters ring 3 and runs no tasks.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::smp_boot::ApBootArgs;

/// Pages the trampoline uses: code + data, PML4, PDPT, PD.
const TRAMP_PAGES: u64 = 4;
/// Preferred trampoline base (SIPI vector 0x08).
const TRAMP_PREFERRED: u64 = 0x8000;
/// Offsets of the data block inside the code page (see the assembly).
const TR_PML4: usize = 0xF00;
const TR_CR4: usize = 0xF04;
const TR_EFER: usize = 0xF08;
const TR_CR0: usize = 0xF0C;
const TR_ENTRY: usize = 0xF10;
const TR_ARGS: usize = 0xF18;
const TR_STAGE: usize = 0xF20;

/// Physical base of the installed trampoline (0 = not installed).
static TRAMP_BASE: AtomicU64 = AtomicU64::new(0);

// Real mode -> protected mode -> long mode. Position-independent: it
// derives its physical base from CS and patches its own GDTR and far
// pointer. Assembled into the kernel image and only ever executed from the
// copy below 1 MiB. Data block offsets match the TR_* constants above.
core::arch::global_asm!(
    r#"
    .section .rodata.veridian_ap_trampoline, "a"
    .balign 4096
    .globl veridian_ap_tramp_start
    .globl veridian_ap_tramp_end
    .code16
veridian_ap_tramp_start:
    cli
    cld
    movw    %cs, %ax
    movw    %ax, %ds
    movzwl  %ax, %ebx
    shll    $4, %ebx
    leal    (tr_gdt - veridian_ap_tramp_start)(%ebx), %eax
    movl    %eax, (tr_gdtr - veridian_ap_tramp_start + 2)
    leal    (tr_pm32 - veridian_ap_tramp_start)(%ebx), %eax
    movl    %eax, (tr_far32 - veridian_ap_tramp_start)
    movl    $1, 0xF20
    lgdtl   (tr_gdtr - veridian_ap_tramp_start)
    movl    %cr0, %eax
    orl     $1, %eax
    movl    %eax, %cr0
    ljmpl   *(tr_far32 - veridian_ap_tramp_start)

    .code32
tr_pm32:
    movw    $0x10, %ax
    movw    %ax, %ds
    movw    %ax, %es
    movw    %ax, %ss
    leal    0xF00(%ebx), %esp
    movl    $2, 0xF20(%ebx)
    movl    0xF04(%ebx), %eax
    movl    %eax, %cr4
    movl    0xF00(%ebx), %eax
    movl    %eax, %cr3
    movl    $0xC0000080, %ecx
    movl    0xF08(%ebx), %eax
    xorl    %edx, %edx
    wrmsr
    movl    0xF0C(%ebx), %eax
    movl    %eax, %cr0
    leal    (tr_lm64 - veridian_ap_tramp_start)(%ebx), %eax
    pushl   $0x18
    pushl   %eax
    lret

    .code64
tr_lm64:
    movl    %ebx, %ebx
    movl    $3, 0xF20(%rbx)
    movq    0xF18(%rbx), %rdi
    movq    0xF10(%rbx), %rax
    jmpq    *%rax

    .balign 8
tr_gdt:
    .quad 0x0000000000000000
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
    .quad 0x00AF9A000000FFFF
tr_gdtr:
    .word tr_gdtr - tr_gdt - 1
    .long 0
tr_far32:
    .long 0
    .word 0x08
veridian_ap_tramp_end:
    .text
    "#,
    options(att_syntax)
);

extern "C" {
    static veridian_ap_tramp_start: u8;
    static veridian_ap_tramp_end: u8;
}

/// Higher-half 64-bit entry (still on the trampoline's page table): switch
/// to the kernel page table and CR4, take the stack from the boot args, and
/// call into Rust with `rdi` = the boot args.
#[unsafe(naked)]
#[no_mangle]
unsafe extern "C" fn veridian_ap_entry64() -> ! {
    core::arch::naked_asm!(
        "mov rax, [rdi + 0x18]",
        "mov cr3, rax",
        "mov rax, [rdi + 0x20]",
        "mov cr4, rax",
        "mov rsp, [rdi + 0x08]",
        "xor ebp, ebp",
        "call {main}",
        "ud2",
        main = sym ap_rust_entry,
    );
}

/// Rust entry of an AP: its per-CPU block, then its own tables, then the
/// common bring-up.
extern "C" fn ap_rust_entry(args: &'static ApBootArgs) -> ! {
    let cpu = args.cpu_id.load(Ordering::Acquire) as usize;
    // SAFETY: CPUID leaf 1 is unprivileged and side-effect free.
    let apic_id = unsafe { core::arch::x86_64::__cpuid(1).ebx >> 24 };
    // SAFETY: first Rust code on this CPU; `cpu` is the logical id the boot
    // CPU assigned it, and nothing else touches that block now.
    unsafe { crate::arch::percpu::install(cpu, apic_id) };
    crate::arch::smp_boot::ap_main()
}

/// Local APIC IDs of the enabled application processors, in MADT order.
pub fn enumerate_secondaries() -> Vec<u32> {
    let mut cpus = Vec::new();
    if super::apic::x2apic_enabled() {
        crate::println!("[SMP] Local APIC is in x2APIC mode (unsupported): APs not started");
        return cpus;
    }
    let bsp = super::apic::read_id().unwrap_or(0);
    super::acpi::with_acpi_info(|info| {
        for lapic in info.local_apics.iter().flatten() {
            // Bit 0 (Enabled) only: an Online Capable entry without it is
            // an empty hot-plug slot that would just time out.
            if lapic.flags & 1 != 0 && lapic.apic_id != bsp {
                cpus.push(u32::from(lapic.apic_id));
            }
        }
    });
    cpus
}

/// A usable, SIPI-addressable base for the trampoline below 1 MiB. The
/// kernel's frame allocator never hands out memory there.
fn find_trampoline_base() -> Option<u64> {
    let ok = |b: u64| {
        !(0xA0..=0xBF).contains(&(b >> 12))
            && super::boot::phys_range_usable(b, b + TRAMP_PAGES * 4096)
    };
    if ok(TRAMP_PREFERRED) {
        return Some(TRAMP_PREFERRED);
    }
    (1..0x9C).map(|p| p << 12).find(|&b| ok(b))
}

/// Copy the trampoline below 1 MiB and build its page tables (once).
fn install_trampoline() -> Option<u64> {
    let base = TRAMP_BASE.load(Ordering::Acquire);
    if base != 0 {
        return Some(base);
    }
    let base = find_trampoline_base()?;
    let virt = |pa: u64| crate::mm::phys_to_virt_addr(pa) as *mut u8;
    // The two symbols delimit the trampoline in kernel rodata (only their
    // addresses are taken).
    let start = core::ptr::addr_of!(veridian_ap_tramp_start);
    let len = core::ptr::addr_of!(veridian_ap_tramp_end) as usize - start as usize;
    if len > TR_PML4 {
        crate::println!("[SMP] AP trampoline too large ({} bytes)", len);
        return None;
    }
    let kernel_l4 = x86_64::registers::control::Cr3::read()
        .0
        .start_address()
        .as_u64();
    // SAFETY: `base..base + 4 pages` is usable RAM below 1 MiB that nothing
    // else uses (find_trampoline_base), reached through the physical map;
    // the kernel L4 is the live page table, read only.
    unsafe {
        core::ptr::write_bytes(virt(base), 0, (TRAMP_PAGES * 4096) as usize);
        core::ptr::copy_nonoverlapping(start, virt(base), len);
        let pml4 = virt(base + 0x1000) as *mut u64;
        let pdpt = virt(base + 0x2000) as *mut u64;
        let pd = virt(base + 0x3000) as *mut u64;
        // Identity map 0-2 MiB (one 2 MiB page) so the instruction after
        // CR0.PG is fetched from the trampoline's own physical address.
        *pml4 = (base + 0x2000) | 0x3;
        *pdpt = (base + 0x3000) | 0x3;
        *pd = 0x83; // present | writable | PS
                    // The kernel half, so the jump to veridian_ap_entry64 resolves.
        let l4 = virt(kernel_l4) as *const u64;
        for i in 256..512 {
            *pml4.add(i) = *l4.add(i);
        }
        let data = virt(base);
        let put32 = |off: usize, v: u32| (data.add(off) as *mut u32).write_unaligned(v);
        let put64 = |off: usize, v: u64| (data.add(off) as *mut u64).write_unaligned(v);
        put32(TR_PML4, (base + 0x1000) as u32);
        put32(TR_CR4, 1 << 5); // PAE
                               // EFER without LMA (read-only), as Linux's trampoline header does.
        let efer = x86_64::registers::model_specific::Efer::read_raw();
        put32(TR_EFER, (efer & !(1 << 10)) as u32);
        put32(TR_CR0, x86_64::registers::control::Cr0::read_raw() as u32);
        put64(TR_ENTRY, veridian_ap_entry64 as *const () as u64);
        put64(
            TR_ARGS,
            &crate::arch::smp_boot::AP_BOOT_ARGS as *const ApBootArgs as u64,
        );
        put32(TR_STAGE, 0);
    }
    TRAMP_BASE.store(base, Ordering::Release);
    crate::println!("[SMP] AP trampoline at {:#x}", base);
    Some(base)
}

/// The kernel page table and CR4 the AP switches to in
/// `veridian_ap_entry64`.
pub fn prepare_boot_args(args: &ApBootArgs) {
    let cr3 = x86_64::registers::control::Cr3::read()
        .0
        .start_address()
        .as_u64();
    args.kernel_cr3.store(cr3, Ordering::Relaxed);
    let cr4 = x86_64::registers::control::Cr4::read_raw();
    args.cr4.store(cr4, Ordering::Relaxed);
}

/// Start the AP with Local APIC ID `hw`.
pub fn start_ap(hw: u32, _args: &'static ApBootArgs) -> Result<(), &'static str> {
    let base = install_trampoline().ok_or("no usable trampoline page below 1 MiB")?;
    // SAFETY: the stage word is in the installed trampoline page.
    unsafe {
        (crate::mm::phys_to_virt_addr(base) as *mut u8)
            .add(TR_STAGE)
            .cast::<u32>()
            .write_volatile(0)
    };
    super::apic::start_ap(hw as u8, (base >> 12) as u8)
}

/// Trampoline stage the last AP reached (1 real, 2 protected, 3 long mode),
/// for diagnosing a timeout.
pub fn trampoline_stage() -> u32 {
    let base = TRAMP_BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    // SAFETY: as in `start_ap`.
    unsafe {
        (crate::mm::phys_to_virt_addr(base) as *const u8)
            .add(TR_STAGE)
            .cast::<u32>()
            .read_volatile()
    }
}

/// This AP's local setup.
pub fn ap_init() {
    crate::arch::smp_boot::ap_stage(10);
    super::gdt::init_ap();
    super::idt::init();
    super::trap::enable_machine_check();
    super::pat::init();
    crate::arch::smp_boot::ap_stage(11);
    super::apic::init_local_secondary();
    super::apic::start_local_timer();
    crate::arch::smp_boot::ap_stage(12);
}

/// Halt with interrupts enabled.
pub fn idle() -> ! {
    loop {
        x86_64::instructions::interrupts::enable_and_hlt();
    }
}
