# Redox Capability–FD Bridge: Notes for VeridianOS Compatibility

## What Redox Is Building
- **FDs become capabilities:** Redox is replacing POSIX file descriptors with capability descriptors; POSIX-style FDs sit on top for compatibility.citeturn0search16turn0search17
- **Capability namespaces & Capsicum-style path:** NLnet/NGI-funded track is making capability routing the default and adding Capsicum-like delegation.citeturn0search13turn0search17

## Bridge Shape for VeridianOS
Goal: allow Veridian user space to interoperate with Redox-style capability transports (and vice versa) without losing Veridian’s capability invariants.

1) **Descriptor Model**
   - Keep Veridian capabilities as the authoritative handle.
   - Expose a *cap-fd shim* that issues small integers referencing capability slots; mirroring Redox’s POSIX-over-cap table keeps libc and tools happy.
   - Use two tables per process (POSIX + cap), matching Redox split to simplify porting.

2) **Transfer Semantics**
   - Support SCM_RIGHTS-equivalent over UNIX domain sockets by shuttling capability IDs through a dedicated control channel; bulk-send batching (vector of caps) to mirror Redox UDS bulk FD passing.
   - For same-address-space threads, prefer shared cap table with refcounts; cross-process requires explicit regrant with rights mask.

3) **Validation & Rights**
   - Enforce explicit right sets on transfer (read/write/ioctl/execute), default-deny on unwrap.
   - Disallow ambient lookup: receiver only gets what was attached.

4) **Kernel Hooks Needed**
   - `sendmsg/recvmsg` ancillary path that accepts a cap slice, checks rights, installs into receiver’s POSIX+cap tables, returns remapped ints.
   - Capability-aware `openat`/`dup` variants that can target either table.

5) **Compatibility Layer**
   - For toolchains expecting plain FDs: keep stable numbering, never expose capability IDs directly.
   - Provide a shim library (`libveri_capfd`) mirroring Redox’s relibc helpers to ease cross-builds.

## Testing Strategy
- UDS round-trip: send N caps (mix of files, sockets, pipes) → verify rights and numeric FD continuity.
- Stress bulk transfer (Wayland-style): hundreds of caps in a single message.
- Negative: attempt transfer without grantable rights; ensure receiver gets EBADF/EACCES.

## QEMU/User Guidance
- On AArch64/RISC-V use virtio-mmio (`-device virtio-blk-device`) so userland has disk to exercise cap passing.
- Ensure `sendmsg/recvmsg` are wired in libc tests for both POSIX and capability handles.
