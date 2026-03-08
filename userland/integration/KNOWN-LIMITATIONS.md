# KDE Plasma 6 on VeridianOS -- Known Limitations

**Version**: Phase 10 Complete (v0.23.0)
**Last Updated**: March 2026

---

## Remaining Limitations

### GPU & Rendering

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No real GPU drivers | Only VirtIO GPU supported; no AMD/NVIDIA/Intel native drivers | Use VirtIO GPU with virgl in QEMU |
| OpenGL ES 2.0 only | No OpenGL 3.x/4.x core profile support | KWin's GLES2 backend handles this transparently |
| Complex script rendering | Arabic/Devanagari ligature rendering may have edge cases | Install additional fonts; Latin/CJK fully supported |

### Data & Sync

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No full Akonadi sync | Local PIM data only; no Exchange/IMAP/CalDAV sync | Use web-based email/calendar; local contacts/notes work |

### Platform

| Limitation | Details | Workaround |
|-----------|---------|------------|
| Multi-arch KDE | KDE Plasma 6 is x86_64 only; AArch64/RISC-V are future work | Kernel + built-in DE run on all 3 architectures |
| Cross-compilation required | All KDE components must be cross-compiled from a host | Use provided build scripts in each component directory |
| No native package manager | KDE libraries installed manually to sysroot | Use `build-kde-rootfs.sh` for full rootfs |

### Performance

| Limitation | Details | Workaround |
|-----------|---------|------------|
| Memory: ~800MB baseline | Plasma session uses 600-800 MB RSS | Allocate 2+ GB RAM to QEMU |
| Font rendering | FreeType + HarfBuzz shims; complex scripts may render incorrectly | Install additional fonts |

## System Requirements

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| RAM | 1 GB | 2 GB+ |
| Disk | 2 GB rootfs | 4 GB+ (for user data) |
| CPU | 1 core | 4 cores (SMP) |
| GPU | VirtIO 2D | VirtIO 3D (virgl) |
| Architecture | x86_64 only | x86_64 (AArch64/RISC-V future) |

---

## Resolved in Phase 10

The following limitations from Phase 9 were resolved in Phase 10 (v0.23.0):

| Limitation | Sprint | Resolution |
|-----------|--------|------------|
| No VSync on llvmpipe | 10.0 | Software VSync via TSC timer + damage tracking (~30+ FPS) |
| Software rendering ~15 FPS | 10.0 | Damage-region recomposite skips unchanged surfaces |
| No PulseAudio/PipeWire | 10.1 | PipeWire daemon with ALSA bridge + PulseAudio compat layer |
| No NetworkManager | 10.2 | NetworkManager shim with Wi-Fi/Ethernet/DNS backends |
| No Bluetooth stack | 10.3 | BlueZ shim with HCI bridge and pairing agent |
| No GLX | 10.4 | GLX-over-EGL translation layer for X11 apps |
| Input methods (XWayland) | 10.4 | XIM-to-Wayland text-input-v3 bridge |
| DRI3 limited | 10.4 | DRI3 extension with GBM buffer management |
| Clipboard large objects | 10.4 | INCR protocol for transfers >256KB |
| No power management | 10.5 | ACPI S3/S4 suspend/hibernate, CPU frequency governors |
| No suspend/hibernate | 10.5 | ACPI S3/S4 state transitions via sysfs interface |
| No screen lock | 10.5/10.6 | DPMS display blanking + PAM-based lock screen |
| No KRunner | 10.6 | KRunner with 6 runners (app/file/calc/cmd/web/unit) |
| No Baloo file indexer | 10.6 | Baloo backend with filesystem crawler + trigram index |
| No Activities | 10.6 | Activities framework with create/switch/delete |
| No webcam/V4L2 | 10.7 | V4L2 device interface with test pattern generator |
| No USB hotplug | 10.7 | xHCI port status polling + udev device daemon |
| Single monitor only | 10.7 | Multi-output manager with EDID parsing |
| No multi-user sessions | 10.8 | Multi-user login via display manager with VT switching |
| No session save/restore | 10.8 | Window state save/restore on logout/login |
| No Akonadi/PIM | 10.8 | Akonadi local data store (contacts/calendar/notes) |
| D-Bus overhead | 10.9 | Message batching + binary shortcut protocol |

---

## Reporting Issues

File issues at: https://github.com/doublegate/VeridianOS/issues

Include:
- QEMU command line used
- Serial console output (`-serial file:serial.log`)
- Screenshot via QMP (`screendump` command)
- KWin log (`/tmp/plasma-session/kwin.log`)
- Session log (`/tmp/plasma-session/plasma-session-*.log`)
