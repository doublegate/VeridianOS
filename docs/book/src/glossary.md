# Glossary

This glossary defines key terms and concepts used throughout the VeridianOS documentation.

## A

**Address Space**
: Complete range of memory addresses available to a process, including code, data, heap, and stack.

**ASLR (Address Space Layout Randomization)**
: Security technique that randomizes memory addresses to prevent exploitation.

**ABI (Application Binary Interface)**
: Low-level interface between applications and the operating system, defining calling conventions and data structures.

**Allocator**
: Component responsible for managing memory allocation and deallocation. VeridianOS uses a hybrid buddy/bitmap allocator.

**Anonymous Memory**
: Memory not backed by a file, typically used for program heap and stack.

**Atomic Operation**
: Operation that completes entirely or not at all, without interruption from other threads.

## B

**Bitmap Allocator**
: Memory allocator that uses a bitmap to track free/used pages, efficient for small allocations.

**Bootloader**
: Program that loads the operating system kernel into memory and transfers control to it.

**Buddy Allocator**
: Memory allocator that divides memory into power-of-two sized blocks, efficient for large allocations.

## C

**Capability**
: Unforgeable token that grants specific access rights to system resources. Core security primitive in VeridianOS.

**Cache Line**
: Smallest unit of data transfer between CPU cache and main memory, typically 64 bytes.

**Context Switch**
: Process of saving the state of one thread and loading the state of another.

**Copy-on-Write (CoW)**
: Optimization technique where copies are deferred until modification occurs.

**CXL (Compute Express Link)**
: High-speed interconnect for processors, memory, and accelerators.

## D

**Demand Paging**
: Memory management technique where pages are loaded only when accessed.

**DMA (Direct Memory Access)**
: Hardware feature allowing devices to access memory without CPU involvement.

**DMA Zone**
: Low memory region (0-16MB) reserved for devices that can only access limited address ranges.

**DPDK (Data Plane Development Kit)**
: Framework for fast packet processing, bypassing the kernel network stack.

**Dirty Page**
: Memory page that has been modified since it was loaded from disk or created.

## E

**eBPF (extended Berkeley Packet Filter)**
: Technology for running sandboxed programs in kernel space.

**ELF (Executable and Linkable Format)**
: Standard file format for executables, object code, and shared libraries.

## F

**Frame**
: Physical memory page, typically 4KB in size.

**Frame Allocator**
: Kernel component that manages physical memory frames.

## G

**GDT (Global Descriptor Table)**
: x86 data structure defining memory segments and their access permissions.

**Grace Period**
: In RCU, the time during which old data must remain valid for existing readers.

## H

**HSM (Hardware Security Module)**
: Dedicated cryptographic processor for managing digital keys.

**Heap**
: Region of memory used for dynamic allocation during program execution.

**Huge Pages**
: Large memory pages (2MB or 1GB) that reduce TLB pressure.

**Hypervisor**
: Software layer that creates and manages virtual machines.

## I

**IDT (Interrupt Descriptor Table)**
: x86 data structure defining interrupt and exception handlers.

**IOMMU (Input-Output Memory Management Unit)**
: Hardware unit providing memory protection and address translation for DMA.

**IPC (Inter-Process Communication)**
: Mechanisms for processes to communicate and share data.

**io_uring**
: Linux asynchronous I/O interface providing high-performance, low-latency I/O operations.

## J

**JIT (Just-In-Time)**
: Compilation technique where code is compiled during execution rather than before.

## K

**KVM (Kernel-based Virtual Machine)**
: Virtualization infrastructure turning Linux kernel into a hypervisor.

**Kernel**
: Core component of the operating system managing hardware resources.

## L

**Lock-Free**
: Programming technique using atomic operations instead of locks for thread synchronization.

**LSM (Linux Security Module)**
: Framework for implementing security policies in the kernel.

## M

**MAC (Mandatory Access Control)**
: Security model where access rules are enforced by the system, not users.

**Microkernel**
: Kernel architecture with minimal functionality in kernel space, most services in user space.

**MLS (Multi-Level Security)**
: Security model with hierarchical classification levels and need-to-know categories.

**MMIO (Memory-Mapped I/O)**
: Technique where device registers appear as memory addresses.

## N

**NUMA (Non-Uniform Memory Access)**
: System architecture where memory access time depends on memory location relative to processor.

**NVMe (Non-Volatile Memory Express)**
: Protocol for accessing solid-state drives via PCIe bus.

## O

**OCI (Open Container Initiative)**
: Industry standards for container formats and runtimes.

## P

**Page**
: Unit of virtual memory, typically 4KB in size.

**Page Fault**
: Exception raised when accessing unmapped or invalid memory, handled by the kernel.

**Page Frame**
: Physical memory page that can be mapped to virtual addresses.

**Page Mapper**
: Kernel component that manages virtual-to-physical page mappings.

**Page Table**
: Data structure mapping virtual addresses to physical addresses.

**Page Table Entry (PTE)**
: Individual entry in a page table containing physical address and permission bits.

**PCB (Process Control Block)**
: Data structure containing information about a process.

**PCR (Platform Configuration Register)**
: TPM register storing measurements for secure boot.

**PML4 (Page Map Level 4)**
: Top-level page table structure in x86_64 architecture.

**POSIX (Portable Operating System Interface)**
: Standards defining API for Unix-like operating systems.

## Q

**QEMU**
: Machine emulator and virtualizer used for testing VeridianOS.

**QoS (Quality of Service)**
: Performance guarantees for system resources.

## R

**RCU (Read-Copy-Update)**
: Synchronization mechanism allowing concurrent reads with updates.

**RDMA (Remote Direct Memory Access)**
: Network protocol for direct memory access between computers.

**Ring Buffer**
: Fixed-size buffer with wrap-around, used for lock-free communication.

**RSS (Receive Side Scaling)**
: Network driver technology distributing packets across CPU cores.

## S

**Scheduler**
: Kernel component deciding which thread runs on which CPU core.

**Secure Boot**
: Boot process verifying each component's digital signature.

**SEV-SNP (Secure Encrypted Virtualization - Secure Nested Paging)**
: AMD technology for encrypted virtual machines.

**SIMD (Single Instruction, Multiple Data)**
: CPU instructions operating on multiple data points simultaneously.

**Slab Allocator**
: Memory allocator that pre-allocates objects of specific sizes to reduce fragmentation.

**SR-IOV (Single Root I/O Virtualization)**
: Technology allowing single PCIe device to appear as multiple devices.

**Swap**
: Process of moving memory pages between RAM and disk storage.

**Syscall (System Call)**
: Interface for user programs to request kernel services.

## T

**TCB (Trusted Computing Base)**
: Set of all hardware and software components critical to security.

**TDX (Trust Domain Extensions)**
: Intel technology for confidential computing.

**TLB (Translation Lookaside Buffer)**
: CPU cache for virtual-to-physical address translations.

**TPM (Trusted Platform Module)**
: Secure cryptoprocessor for hardware-based security.

**TSC (Time Stamp Counter)**
: CPU register counting processor cycles, used for high-resolution timing.

## U

**UEFI (Unified Extensible Firmware Interface)**
: Modern firmware interface replacing BIOS.

**Unikernel**
: Specialized OS kernel compiled with application into single executable.

## V

**VFS (Virtual File System)**
: Abstraction layer providing uniform interface to different file systems.

**VFIO (Virtual Function I/O)**
: Framework for secure device access from user space.

**Virtual Address**
: Memory address in a process's virtual address space, translated to physical address by MMU.

**Virtual Memory**
: Memory management technique providing each process with its own address space.

**VMA (Virtual Memory Area)**
: Contiguous range of virtual addresses with same permissions.

**VMX (Virtual Machine Extensions)**
: Intel CPU virtualization technology.

## W

**Wayland**
: Modern display server protocol replacing X11.

## X

**XDP (eXpress Data Path)**
: High-performance packet processing framework in Linux kernel.

**xHCI (eXtensible Host Controller Interface)**
: USB 3.0 host controller specification.

## Z

**Zero-Copy**
: Data transfer technique avoiding unnecessary copying between buffers.

**Zeroize**
: Securely erasing sensitive data from memory.

## Acronym Quick Reference

| Acronym | Full Form |
|---------|-----------|
| API | Application Programming Interface |
| CPU | Central Processing Unit |
| DMA | Direct Memory Access |
| GPU | Graphics Processing Unit |
| HAL | Hardware Abstraction Layer |
| IRQ | Interrupt Request |
| MMU | Memory Management Unit |
| NIC | Network Interface Card |
| OS | Operating System |
| PCI | Peripheral Component Interconnect |
| PID | Process Identifier |
| RAM | Random Access Memory |
| ROM | Read-Only Memory |
| SMP | Symmetric Multiprocessing |
| TID | Thread Identifier |
| UID | User Identifier |
| VM | Virtual Machine |

## VeridianOS-Specific Terms

**Capability Token**
: 64-bit value encoding object ID, access rights, and version for secure resource access.

**Hybrid Allocator**
: VeridianOS memory allocator combining buddy system for large allocations with bitmap for small ones.

**Microkernel Core**
: Minimal kernel containing only memory management, scheduling, IPC, and capability enforcement.

**Three-Layer IPC**
: VeridianOS IPC architecture with POSIX, translation, and native layers for compatibility and performance.

**User Space Driver**
: Device driver running as unprivileged process, improving security and stability.

**VeridianFS**
: Native copy-on-write file system with compression, deduplication, and snapshots.

**Zero-Copy IPC**
: Message passing using shared memory mappings to avoid data copying.