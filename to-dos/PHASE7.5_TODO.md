# Phase 7.5: Follow-On Enhancements TODO

**Phase Duration**: 6-12 months
**Status**: Planned
**Dependencies**: Phase 7 complete (v0.10.0)
**Last Updated**: February 28, 2026

## Overview

Phase 7.5 covers follow-on enhancements to Phase 7 subsystems. These items were identified during Phase 7 development as natural extensions that require the Phase 7 foundation but were not critical for the initial production readiness milestone. Items are organized into 13 categories spanning networking, multimedia, GPU, hypervisor, containers, security, performance, desktop, userland, filesystem, protocols, and hardware drivers.

---

## 1. Networking Enhancements

- [ ] TCP congestion control algorithms (Reno, Cubic)
- [ ] TCP Selective Acknowledgment (SACK)
- [ ] DNS resolver (recursive queries, caching, /etc/resolv.conf)
- [ ] HTTP/1.1 client library (chunked transfer, keep-alive, redirects)
- [ ] VLAN (802.1Q) tagging and trunk support
- [ ] Multicast group management (IGMP/MLD)
- [ ] NIC bonding / link aggregation (active-backup, round-robin)

---

## 2. Audio Enhancements

- [ ] ALSA-compatible user-space API (PCM open/close/read/write, mixer controls)
- [ ] USB Audio Class driver (UAC 1.0/2.0)
- [ ] HDMI audio output via GPU driver integration
- [ ] Audio recording / capture pipeline
- [ ] OGG Vorbis decoder
- [ ] MP3 decoder (minimp3 or equivalent)
- [ ] Real-time audio scheduling (deadline priority for audio threads)

---

## 3. Video Enhancements

- [ ] PNG decoder (DEFLATE decompression, IDAT chunk handling, interlacing)
- [ ] JPEG decoder (baseline DCT, Huffman, quantization)
- [ ] GIF decoder (LZW decompression, animation frame sequencing)
- [ ] AVI container parser (RIFF/AVI, audio/video stream demux)
- [ ] Frame rate conversion (frame duplication, interpolation)
- [ ] Subtitle overlay (SRT text rendering on video frames)

---

## 4. GPU Acceleration

- [ ] VirtIO GPU 3D (virgl protocol, 3D resource creation, command submission)
- [ ] OpenGL ES 2.0 software rasterizer (vertex/fragment shaders, texture sampling)
- [ ] GEM/TTM buffer management (GPU memory allocation, cache coherency)
- [ ] DRM KMS (Kernel Mode Setting) interface for user-space display servers
- [ ] Vsync / page flip support (vblank events, double buffering)
- [ ] Hardware cursor plane (GPU cursor overlay, position updates)

---

## 5. Hypervisor Enhancements

- [ ] Nested virtualization (L2 VMCS shadowing)
- [ ] VirtIO device passthrough to guests
- [ ] Live migration (VMCS serialization, memory pre-copy, stop-and-copy)
- [ ] Guest SMP support (multi-vCPU with virtual LAPIC)
- [ ] Virtual LAPIC emulation (timer, IPI delivery, EOI)
- [ ] VM snapshots (VMCS + memory + device state serialization)

---

## 6. Container Enhancements

- [ ] OCI runtime specification compliance (config.json, rootfs, lifecycle hooks)
- [ ] Container image format (layer extraction, overlayfs composition)
- [ ] Cgroup memory controller (limit, usage tracking, OOM notification)
- [ ] Cgroup CPU controller (shares, quota, period enforcement)
- [ ] Overlay filesystem (lower/upper/work layers, copy-up, whiteout files)
- [ ] Veth networking (virtual Ethernet pairs, bridge, NAT)
- [ ] Seccomp BPF (syscall filtering, allow/deny/trace actions)

---

## 7. Security

- [ ] KASLR (Kernel Address Space Layout Randomization)
- [ ] Stack canaries (function prologue/epilogue guard values)
- [ ] SMEP/SMAP enforcement (Supervisor Mode Execution/Access Prevention)
- [ ] Spectre retpoline mitigation for indirect branches
- [ ] Capability revocation propagation (transitive revocation tree walk)
- [ ] Audit log persistence (write-ahead log to BlockFS, rotation)

---

## 8. Performance

- [ ] Deadline scheduling (Earliest Deadline First with APIC timer integration)
- [ ] Cache-aware memory allocation (L1/L2/L3 topology detection, coloring)
- [ ] False sharing elimination (cache line padding, per-CPU alignment)
- [ ] Power management (C-states, P-states via ACPI _CST/_PSS methods)
- [ ] Profile-guided optimization (instrumentation, PGO build pipeline)

---

## 9. Desktop

- [ ] Clipboard protocol (wl_data_device, MIME type negotiation, paste)
- [ ] Drag-and-drop (wl_data_offer, enter/leave/drop events)
- [ ] Global keyboard shortcuts (configurable key bindings, shortcut manager)
- [ ] Theme engine (color schemes, icon themes, GTK/Qt style compat)
- [ ] Font rendering (TrueType/OpenType rasterizer, hinting, subpixel)
- [ ] CJK Unicode support (wide character rendering, input method framework)

---

## 10. Shell and Userland

- [ ] io_uring for user-space async I/O (submission/completion queues)
- [ ] ptrace system call (PTRACE_ATTACH, PTRACE_PEEKDATA, single-step)
- [ ] Core dump generation (ELF core format, register state, memory segments)
- [ ] User and group management (/etc/passwd, /etc/group, useradd/userdel)
- [ ] sudo/su privilege elevation (PAM-style authentication, policy files)
- [ ] Crontab scheduler (cron daemon, crontab parsing, job execution)

---

## 11. Filesystem

- [ ] ext4 read-only support (extent tree, dir_index, journal replay)
- [ ] FAT32 read/write (long file names, directory traversal, cluster chains)
- [ ] tmpfs (memory-backed filesystem with size limits)
- [ ] inotify (file system event monitoring, watch descriptors)
- [ ] File locking (flock, fcntl F_SETLK/F_GETLK, POSIX advisory locks)
- [ ] Extended attributes (xattr get/set/list/remove, user/system namespace)

---

## 12. Networking Protocols

- [ ] QUIC protocol (UDP-based transport, TLS 1.3 integration, stream multiplexing)
- [ ] WireGuard VPN (Noise protocol framework, ChaCha20-Poly1305 tunnel)
- [ ] mDNS/DNS-SD (multicast service discovery, .local resolution)
- [ ] NTP client (time synchronization, clock discipline, stratum tracking)
- [ ] SSH server (Ed25519 host keys, channel multiplexing, shell session)
- [ ] TLS 1.3 (handshake state machine, AEAD encryption, certificate validation)

---

## 13. Hardware Drivers

- [ ] USB xHCI host controller (command/event/transfer rings, device slots)
- [ ] USB mass storage (bulk-only transport, SCSI command set)
- [ ] USB HID (keyboard/mouse via interrupt transfers, report descriptors)
- [ ] Bluetooth HCI (command/event transport, L2CAP, SDP)
- [ ] AHCI/SATA controller (FIS-based I/O, command list, port multiplier)
- [ ] RTC (CMOS real-time clock, alarm, century register)

---

## Progress Tracking

| Category | Items | Completed | Status |
|----------|-------|-----------|--------|
| Networking Enhancements | 7 | 0 | Planned |
| Audio Enhancements | 7 | 0 | Planned |
| Video Enhancements | 6 | 0 | Planned |
| GPU Acceleration | 6 | 0 | Planned |
| Hypervisor Enhancements | 6 | 0 | Planned |
| Container Enhancements | 7 | 0 | Planned |
| Security | 6 | 0 | Planned |
| Performance | 5 | 0 | Planned |
| Desktop | 6 | 0 | Planned |
| Shell/Userland | 6 | 0 | Planned |
| Filesystem | 6 | 0 | Planned |
| Networking Protocols | 6 | 0 | Planned |
| Hardware Drivers | 6 | 0 | Planned |
| **Total** | **~80** | **0** | **0%** |

---

**Previous Phase**: [Phase 7 - Production Readiness](PHASE7_TODO.md)
**Next Phase**: [Phase 8 - Next-Generation Features](PHASE8_TODO.md)
**See Also**: [Master TODO](MASTER_TODO.md)
