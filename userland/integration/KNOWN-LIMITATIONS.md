# KDE Plasma 6 on VeridianOS -- Known Limitations

**Version**: Phase 9 Sprint 9.10 (Integration + Polish)
**Last Updated**: March 2026

---

## Rendering

| Limitation | Details | Workaround |
|-----------|---------|------------|
| Software rendering | llvmpipe provides ~15 FPS; usable but not smooth | Use VirtIO GPU with virgl (`--enable-virgl`) in QEMU for 60 FPS |
| No real GPU support | Only VirtIO GPU is supported; no AMD/NVIDIA/Intel drivers | Future: DRM/KMS drivers for real hardware |
| No VSync on llvmpipe | Screen tearing possible with software rendering | Switch to virgl or accept tearing |
| OpenGL ES 2.0 only | No OpenGL 3.x/4.x core profile support | KWin's GLES2 backend handles this transparently |

## XWayland

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No GLX | X11 apps cannot use GLX for OpenGL; only EGL via glamor | Use Wayland-native apps when possible |
| Input methods | CJK/IBus/Fcitx not fully wired through XWayland | Use Wayland-native input method framework |
| DRI3 limited | Hardware-accelerated X11 rendering requires VirtIO GPU | Software rendering used as fallback |
| Clipboard large objects | X11 <-> Wayland clipboard limited to 16 MB per transfer | Transfer files via filesystem instead |

## Hardware Support

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No NetworkManager | Network configuration not available through KDE settings | Use manual `ip`/`ifconfig` commands |
| No Bluetooth stack | BlueZ not ported; Bluetooth settings panel non-functional | Use serial/USB for peripherals |
| No PulseAudio/PipeWire | Audio playback and recording not available | Future: audio subsystem port |
| No webcam/V4L2 | Video capture devices not supported | N/A |
| No USB hotplug | USB devices must be present at boot | Restart QEMU to add/remove USB devices |
| Single monitor only | Multi-monitor/display configuration not supported | Use single display output |

## KDE Features

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No Activities | KDE Activities framework disabled | Use virtual desktops instead |
| No screen lock | Screen locking/DPMS not functional | Lock via QEMU monitor or close window |
| No Baloo file indexer | Desktop search and file indexing disabled | Use `find`/`grep` for file search |
| No KRunner | Application launcher search limited | Use application menu |
| No Akonadi/PIM | KDE PIM (email, calendar, contacts) not available | Use web-based alternatives |
| No power management | PowerDevil backend reports but cannot control hardware | Power states managed by QEMU |
| No suspend/hibernate | ACPI S3/S4 states not implemented | Shut down and restart instead |

## Performance

| Limitation | Details | Workaround |
|-----------|---------|------------|
| Memory: ~800MB baseline | Plasma session uses 600-800 MB RSS | Allocate 2+ GB RAM to QEMU |
| Boot time: ~8-12s | KWin startup to desktop in 8-12 seconds | Pre-warm font cache with `fc-cache -f` |
| Font rendering | FreeType + HarfBuzz shims; complex scripts may render incorrectly | Install additional fonts |
| D-Bus overhead | Session bus adds ~0.5-1ms to IPC calls | Acceptable for desktop use |

## System Requirements

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| RAM | 1 GB | 2 GB+ |
| Disk | 2 GB rootfs | 4 GB+ (for user data) |
| CPU | 1 core | 4 cores (SMP) |
| GPU | VirtIO 2D | VirtIO 3D (virgl) |
| Architecture | x86_64 only | x86_64 (AArch64/RISC-V future) |

## Session Management

| Limitation | Details | Workaround |
|-----------|---------|------------|
| No multi-user sessions | Only single-user (root) supported | Run as root; user accounts planned |
| No session save/restore | Window positions not preserved across reboots | Manually re-open applications |
| Session type requires reboot | Switching between built-in DE and KDE requires reboot | Edit `/etc/veridian/session.conf` and reboot |
| Auto-login only | No graphical login screen in initial release | Display manager (veridian-dm) is text-based |

## Build System

| Limitation | Details | Workaround |
|-----------|---------|------------|
| Cross-compilation required | All KDE components must be cross-compiled | Use provided build scripts |
| No native package manager | Libraries installed manually to sysroot | Use `build-kde-rootfs.sh` |
| CI build time | Full KDE sysroot build takes ~90 minutes | Sysroot caching reduces to ~5 minutes |

---

## Reporting Issues

File issues at: https://github.com/doublegate/VeridianOS/issues

Include:
- QEMU command line used
- Serial console output (`-serial file:serial.log`)
- Screenshot via QMP (`screendump` command)
- KWin log (`/tmp/plasma-session/kwin.log`)
- Session log (`/tmp/plasma-session/plasma-session-*.log`)
