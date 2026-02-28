# Phase 8: Next-Generation Features TODO

**Phase Duration**: 12-24 months
**Status**: Planned
**Dependencies**: Phase 7.5 (follow-on enhancements)
**Last Updated**: February 28, 2026

## Overview

Phase 8 covers next-generation features that push VeridianOS toward a fully self-sufficient, enterprise-ready operating system. These items require the complete Phase 7 foundation (hypervisor, containers, security hardening, performance optimization) and Phase 7.5 enhancements (filesystem maturity, networking protocols, hardware drivers). Items are organized into 9 categories spanning web browsing, advanced self-hosting, desktop v2, networking v2, cloud-native, advanced virtualization, enterprise features, developer tools, and formal verification.

---

## 1. Web Browser

- [ ] HTML parser (tokenizer, tree construction, DOM builder)
- [ ] CSS box model (cascade, specificity, inheritance, computed styles)
- [ ] DOM tree (element/text/comment nodes, attribute access, tree traversal)
- [ ] Layout engine (block/inline/flex formatting contexts, float, position)
- [ ] Rendering pipeline (display list, paint, compositing, scrolling)
- [ ] JavaScript interpreter (lexer, parser, bytecode compiler, VM with GC)
- [ ] HTTP + TLS client (HTTP/1.1, HTTPS via TLS 1.3, cookie jar, redirect)
- [ ] Tabbed browsing (multi-tab UI, per-tab process isolation, address bar)

---

## 2. Advanced Self-Hosting

- [ ] Bootstrap rustc natively on VeridianOS (Stage 0 -> Stage 1 -> Stage 2 on-target)
- [ ] Native LLVM build (full LLVM 19+ compilation on VeridianOS)
- [ ] Package build system (automated source -> binary package pipeline)
- [ ] GDB stub (remote debugging protocol, breakpoints, watchpoints, single-step)
- [ ] Rustdoc generation on-target (HTML documentation from source)

---

## 3. Desktop v2

- [ ] GPU-accelerated compositor (OpenGL ES render path, texture atlas, shader-based blending)
- [ ] Desktop icons (icon grid, .desktop file association, drag to launch)
- [ ] File associations (default app registry, "Open With" dialog, MIME preference)
- [ ] Print/PDF support (CUPS-compatible spooler, PDF renderer, print dialog)
- [ ] Accessibility (screen reader API, high contrast mode, keyboard navigation)
- [ ] Display manager (login screen, multi-user session management, session types)

---

## 4. Networking v2

- [ ] Firewall / iptables (packet filter, chains, rules, NAT table, connection tracking)
- [ ] NAT (SNAT/DNAT/masquerade for container and VM networking)
- [ ] Routing daemon (RIP/OSPF basic implementation, route table management)
- [ ] WiFi 802.11 (mac80211 framework, WPA2/WPA3, association, scanning)
- [ ] Bluetooth stack (HCI, L2CAP, RFCOMM, profiles)
- [ ] VPN client (WireGuard/OpenVPN protocol, tunnel interface, key management)

---

## 5. Cloud-Native Platform

- [ ] Kubernetes CRI (Container Runtime Interface, pod lifecycle, image pull)
- [ ] CNI (Container Network Interface, bridge/overlay plugins, IP allocation)
- [ ] CSI (Container Storage Interface, volume mount/unmount, snapshots)
- [ ] Service mesh (sidecar proxy, mTLS, service discovery, load balancing)
- [ ] Load balancer (L4/L7 balancing, health checks, round-robin/least-connections)
- [ ] Cloud-init (instance metadata, user-data scripts, SSH key injection)

---

## 6. Advanced Virtualization

- [ ] Full KVM API compatibility (ioctl interface, vcpu create/run, memory regions)
- [ ] QEMU compatibility layer (device model integration, migration protocol)
- [ ] PCI passthrough via VFIO (IOMMU group isolation, BAR mapping, interrupt remapping)
- [ ] SR-IOV (Virtual Function creation, VF assignment to guests)
- [ ] Hot-plug support (CPU, memory, PCI device hot-add/remove)

---

## 7. Enterprise Features

- [ ] LDAP/Active Directory client (bind, search, authentication)
- [ ] Kerberos authentication (AS-REQ/AS-REP, TGS-REQ/TGS-REP, ticket cache)
- [ ] NFS v4 client (COMPOUND operations, delegation, locking)
- [ ] CIFS/SMB client (SMB2/3 dialect negotiation, share mount, auth)
- [ ] iSCSI initiator (login, discovery, session management, data-out)
- [ ] Software RAID (mdadm-compatible, RAID 0/1/5/6, rebuild, monitoring)

---

## 8. Developer Tools

- [ ] IDE with LSP support (text editor + Language Server Protocol, code actions, diagnostics)
- [ ] Native git client (clone, commit, push, pull, branch, merge, diff)
- [ ] Package repository hosting (HTTP server, package index, upload/download)
- [ ] CI runner (job execution, artifact collection, status reporting)
- [ ] Profiling GUI (flame graph visualization, CPU/memory timeline, call tree)

---

## 9. Formal Verification

- [ ] Verified boot chain (cryptographic measurement, TPM PCR extend, policy engine)
- [ ] Formally verified IPC (model-checked message passing, deadlock freedom proof)
- [ ] Verified memory allocator (allocation/deallocation correctness, no use-after-free proof)
- [ ] Capability formal model (access control lattice, information flow proof)

---

## Progress Tracking

| Category | Items | Completed | Status |
|----------|-------|-----------|--------|
| Web Browser | 8 | 0 | Planned |
| Advanced Self-Hosting | 5 | 0 | Planned |
| Desktop v2 | 6 | 0 | Planned |
| Networking v2 | 6 | 0 | Planned |
| Cloud-Native Platform | 6 | 0 | Planned |
| Advanced Virtualization | 5 | 0 | Planned |
| Enterprise Features | 6 | 0 | Planned |
| Developer Tools | 5 | 0 | Planned |
| Formal Verification | 4 | 0 | Planned |
| **Total** | **~51** | **0** | **0%** |

---

**Previous Phase**: [Phase 7.5 - Follow-On Enhancements](PHASE7.5_TODO.md)
**See Also**: [Master TODO](MASTER_TODO.md)
