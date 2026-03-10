# Phase 10: KDE Known Limitations Remediation

**Phase Duration**: 8-12 months
**Status**: COMPLETE (all 11 sprints)
**Dependencies**: Phase 9 (KDE Plasma 6 porting infrastructure)
**Last Updated**: March 2026
**Version**: v0.23.0

## Overview

Phase 10 systematically remediates the known limitations documented in Phase 9's KNOWN-LIMITATIONS.md. Each sprint addresses a specific limitation category -- from rendering performance and audio through networking, Bluetooth, XWayland enhancements, power management, KDE desktop features, hardware support, session management, and performance optimization. The final sprint (10.10) handles integration testing, CI updates, documentation, and version bump.

**Design Reference**: [KDE Known Limitations](../userland/integration/KNOWN-LIMITATIONS.md)

---

## Sprint 10.0: Rendering Performance -- VSync + Damage Tracking

**Duration**: 3-4 weeks | **Priority**: HIGH | **Blocks**: None

### 10.0.1 Kernel Damage Tracking

- [x] `kernel/src/graphics/damage_tracking.rs` -- Per-surface dirty-rect list, merge algorithm
- [x] `kernel/src/graphics/vsync_sw.rs` -- Software VSync timer via TSC
- [x] `kernel/src/graphics/gpu_accel.rs` -- sw_vsync_flip(), composite_damaged_only()
- [x] `kernel/src/graphics/mod.rs` -- Register new submodules (damage_tracking, vsync_sw)

### 10.0.2 KWin Software Rendering Backend

- [x] `userland/kwin/kwin-veridian-swrender.h` -- Software rendering backend header
- [x] `userland/kwin/kwin-veridian-swrender.cpp` -- Damage-region recomposite, VSync wait
- [x] `userland/kwin/kwin-veridian-platform.cpp` -- Wire llvmpipe path to swrender
- [x] `userland/kwin/build-kwin.sh` -- Add swrender source

---

## Sprint 10.1: PipeWire/PulseAudio Audio Daemon

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Plasma audio applet

### 10.1.1 PipeWire Daemon

- [x] `userland/pipewire/pipewire-veridian.h` -- PipeWire API (pw_init, pw_main_loop, pw_stream)
- [x] `userland/pipewire/pipewire-veridian.cpp` -- PipeWire daemon, stream management, graph routing
- [x] `userland/pipewire/pw-alsa-bridge.h` -- ALSA bridge header
- [x] `userland/pipewire/pw-alsa-bridge.cpp` -- ALSA device access, integer resampler (44.1/48 kHz)

### 10.1.2 PulseAudio Compatibility

- [x] `userland/pipewire/pulseaudio-compat.h` -- PulseAudio API header (pa_context, pa_stream)
- [x] `userland/pipewire/pulseaudio-compat.cpp` -- PA compat layer translating to PipeWire graph
- [x] `userland/pipewire/build-pipewire.sh` -- Build script

### 10.1.3 Plasma Integration

- [x] `userland/plasma/plasma-audio-applet.h` -- Audio applet header
- [x] `userland/plasma/plasma-audio-applet.cpp` -- Volume applet with per-stream control

### 10.1.4 Kernel ALSA Support

- [x] `kernel/src/audio/alsa.rs` -- PCM ioctl dispatch (SNDRV_PCM_IOCTL_*)
- [x] `kernel/src/syscall/filesystem.rs` -- Audio ioctl routing (/dev/snd/*)

---

## Sprint 10.2: NetworkManager Shim

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Plasma network applet

### 10.2.1 NM Daemon

- [x] `userland/networkmanager/nm-veridian.h` -- NM D-Bus API (org.freedesktop.NetworkManager)
- [x] `userland/networkmanager/nm-veridian.cpp` -- NM daemon with connection profiles, device state machine

### 10.2.2 Network Backends

- [x] `userland/networkmanager/nm-wifi.h` -- Wi-Fi backend header
- [x] `userland/networkmanager/nm-wifi.cpp` -- AP scanning, WPA2 authentication
- [x] `userland/networkmanager/nm-ethernet.h` -- Ethernet backend header
- [x] `userland/networkmanager/nm-ethernet.cpp` -- Link detection, DHCP client integration
- [x] `userland/networkmanager/nm-dns.h` -- DNS backend header
- [x] `userland/networkmanager/nm-dns.cpp` -- resolv.conf management, DNS caching
- [x] `userland/networkmanager/build-nm.sh` -- Build script

### 10.2.3 Plasma Integration

- [x] `userland/plasma/plasma-nm-applet.h` -- Network applet header
- [x] `userland/plasma/plasma-nm-applet.cpp` -- Connection list, Wi-Fi scan, VPN status

### 10.2.4 Kernel Netlink IPC

- [x] `kernel/src/net/netlink.rs` -- Netlink-style IPC for kernel<->NM communication

---

## Sprint 10.3: BlueZ Bluetooth

**Duration**: 4-6 weeks | **Priority**: MEDIUM | **Blocks**: Plasma Bluetooth applet

### 10.3.1 BlueZ Daemon

- [x] `userland/bluez/bluez-veridian.h` -- BlueZ D-Bus API (org.bluez.*)
- [x] `userland/bluez/bluez-veridian.cpp` -- BlueZ daemon with adapter/device management

### 10.3.2 HCI Bridge

- [x] `userland/bluez/bluez-hci-bridge.h` -- HCI bridge header
- [x] `userland/bluez/bluez-hci-bridge.cpp` -- HCI command/event handling, connection management

### 10.3.3 Pairing

- [x] `userland/bluez/bluez-pair.h` -- Pairing agent header
- [x] `userland/bluez/bluez-pair.cpp` -- PIN/SSP pairing with Secure Simple Pairing
- [x] `userland/bluez/build-bluez.sh` -- Build script

### 10.3.4 Plasma Integration

- [x] `userland/plasma/plasma-bluetooth-applet.h` -- BT applet header
- [x] `userland/plasma/plasma-bluetooth-applet.cpp` -- Adapter toggle, device list, pairing UI

### 10.3.5 Kernel Bluetooth Device Node

- [x] `kernel/src/drivers/bluetooth/device_node.rs` -- /dev/bluetooth/hci0 device node

---

## Sprint 10.4: XWayland Enhancements

**Duration**: 3-4 weeks | **Priority**: MEDIUM | **Blocks**: None

### 10.4.1 GLX Support

- [x] `userland/integration/xwayland-glx.h` -- GLX 1.4 API header
- [x] `userland/integration/xwayland-glx.cpp` -- GLX-over-EGL translation layer

### 10.4.2 DRI3 Extension

- [x] `userland/integration/xwayland-dri3.h` -- DRI3 header
- [x] `userland/integration/xwayland-dri3.cpp` -- GBM buffer management, fd passing

### 10.4.3 Input Method Bridge

- [x] `userland/integration/xwayland-im.h` -- XIM bridge header
- [x] `userland/integration/xwayland-im.cpp` -- XIM-to-Wayland text-input-v3 translation

### 10.4.4 Rich Clipboard

- [x] `userland/integration/xwayland-veridian.cpp` -- PNG/BMP/URI clipboard, INCR protocol for large transfers
- [x] `userland/integration/xwayland-veridian.h` -- IM forwarding declarations
- [x] `userland/integration/build-xwayland.sh` -- Add GLX, DRI3, IM sources

---

## Sprint 10.5: Power Management

**Duration**: 4-5 weeks | **Priority**: MEDIUM | **Blocks**: PowerDevil backend

### 10.5.1 ACPI Power States

- [x] `kernel/src/arch/x86_64/acpi_pm.rs` -- ACPI S3/S4/S5 state transitions, SCI handler

### 10.5.2 Display Power

- [x] `kernel/src/arch/x86_64/dpms.rs` -- Display Power Management Signaling (on/standby/suspend/off)

### 10.5.3 CPU Frequency Scaling

- [x] `kernel/src/arch/x86_64/cpufreq.rs` -- P-state control, governors (performance/powersave/ondemand)

### 10.5.4 Sysfs Interface

- [x] `kernel/src/sysfs/mod.rs` -- sysfs module registration
- [x] `kernel/src/sysfs/power.rs` -- Virtual files for /sys/class/power_supply, backlight, cpufreq

### 10.5.5 Plasma Integration

- [x] `userland/plasma/powerdevil-veridian-backend.cpp` -- Write operations for suspend, brightness, governor
- [x] `userland/plasma/plasma-veridian-lockscreen.cpp` -- DPMS integration for screen blank on lock

---

## Sprint 10.6: KDE Features

**Duration**: 5-6 weeks | **Priority**: MEDIUM | **Blocks**: None

### 10.6.1 KRunner

- [x] `userland/plasma/krunner-veridian.h` -- KRunner API header
- [x] `userland/plasma/krunner-veridian.cpp` -- App/file/calc/cmd/web search runners (6 total)

### 10.6.2 Baloo File Indexer

- [x] `userland/kf6/baloo-veridian-backend.h` -- Baloo backend header
- [x] `userland/kf6/baloo-veridian-backend.cpp` -- Filesystem crawler, inotify watcher
- [x] `userland/kf6/baloo-veridian-index.h` -- Index storage header
- [x] `userland/kf6/baloo-veridian-index.cpp` -- Inverted index, trigram search

### 10.6.3 Activities Framework

- [x] `userland/plasma/activities-veridian.h` -- Activities header
- [x] `userland/plasma/activities-veridian.cpp` -- Activity management (create, switch, delete)

### 10.6.4 Screen Lock

- [x] `userland/plasma/kscreen-veridian-backend.cpp` -- Screen lock trigger, lid close events
- [x] `userland/plasma/plasma-veridian-lockscreen.cpp` -- PAM auth, DPMS display blank

### 10.6.5 Build Integration

- [x] `userland/plasma/build-plasma-apps.sh` -- Add KRunner, Activities
- [x] `userland/kf6/build-tier3.sh` -- Add Baloo backend

---

## Sprint 10.7: USB Hotplug + V4L2 + Multi-Monitor

**Duration**: 4-5 weeks | **Priority**: MEDIUM | **Blocks**: Solid backend

### 10.7.1 USB Hotplug

- [x] `kernel/src/drivers/usb/hotplug.rs` -- xHCI port status polling, device attach/detach events

### 10.7.2 udev Daemon

- [x] `userland/udev/udev-veridian.h` -- udev daemon header
- [x] `userland/udev/udev-veridian.cpp` -- Device event daemon, rule matching
- [x] `userland/udev/libudev-veridian.h` -- libudev API header
- [x] `userland/udev/libudev-veridian.cpp` -- libudev shim for client queries
- [x] `userland/udev/build-udev.sh` -- Build script

### 10.7.3 V4L2 Video Capture

- [x] `kernel/src/drivers/v4l2.rs` -- V4L2 device interface, test pattern generator

### 10.7.4 Multi-Monitor

- [x] `kernel/src/graphics/multi_output.rs` -- Multi-output manager, EDID parsing
- [x] `userland/kwin/kwin-veridian-platform.cpp` -- Multi-output support in KWin platform

### 10.7.5 Solid Backend

- [x] `userland/kf6/solid-veridian-backend.cpp` -- udev event subscription for Solid device notifications

---

## Sprint 10.8: Session Management

**Duration**: 3-4 weeks | **Priority**: MEDIUM | **Blocks**: None

### 10.8.1 Kernel Session Support

- [x] `kernel/src/process/session.rs` -- Session groups, per-session isolation, VT switching

### 10.8.2 Session Save/Restore

- [x] `userland/plasma/session-save-restore.h` -- Save/restore header
- [x] `userland/plasma/session-save-restore.cpp` -- Window state save/restore on logout/login

### 10.8.3 Akonadi PIM Data Store

- [x] `userland/akonadi/akonadi-veridian.h` -- PIM data store header
- [x] `userland/akonadi/akonadi-veridian.cpp` -- Contacts/calendar/notes (local storage)
- [x] `userland/akonadi/build-akonadi.sh` -- Build script

### 10.8.4 Multi-User Login

- [x] `userland/integration/veridian-dm.cpp` -- Multi-user support in display manager
- [x] `kernel/src/desktop/display_manager.rs` -- Per-user session tracking

### 10.8.5 Session Lifecycle

- [x] `userland/plasma/plasma-veridian-session.sh` -- Session save/restore hooks on start/stop

---

## Sprint 10.9: Performance Optimization

**Duration**: 3-4 weeks | **Priority**: HIGH | **Blocks**: None

### 10.9.1 Memory Optimization

- [x] `userland/integration/plasma-memory-opt.h` -- Memory optimization header
- [x] `userland/integration/plasma-memory-opt.cpp` -- Lazy KF6 plugin loading, cache cleanup

### 10.9.2 D-Bus Optimization

- [x] `userland/integration/dbus-optimize.h` -- D-Bus optimization header
- [x] `userland/integration/dbus-optimize.cpp` -- Message batching, binary shortcut, credential cache

### 10.9.3 Kernel Same-page Merging

- [x] `kernel/src/mm/ksm.rs` -- KSM scanner for identical anonymous pages

### 10.9.4 Startup Optimization

- [x] `userland/integration/veridian-kde-init.sh` -- Parallel daemon startup (PipeWire, NM, BlueZ, udev)
- [x] `userland/integration/kde-perf-profile.sh` -- Memory/boot profiling scripts

### 10.9.5 Session Startup

- [x] `userland/plasma/plasma-veridian-session.sh` -- Parallel service startup order

---

## Sprint 10.10: Integration + Polish

**Duration**: 2-3 weeks | **Priority**: HIGH | **Blocks**: Release

### 10.10.1 CI Pipeline

- [x] `userland/integration/ci-kde-build.yml` -- Add Phase 10 component builds and checks

### 10.10.2 Test Suite

- [x] `userland/integration/kde-test-suite.sh` -- Add 9 test functions for Phase 10 features

### 10.10.3 Documentation

- [x] `userland/integration/KNOWN-LIMITATIONS.md` -- Rewrite with resolved items and remaining gaps
- [x] `to-dos/PHASE10_TODO.md` -- This file
- [x] `to-dos/MASTER_TODO.md` -- Phase 10 entry
- [x] `CHANGELOG.md` -- v0.23.0 release notes
- [x] `README.md` -- Phase 10 features, version bump

### 10.10.4 Version Bump

- [x] `Cargo.toml` -- 0.22.0 -> 0.23.0
- [x] `kernel/src/fs/mod.rs` -- VERSION 0.22.0 -> 0.23.0
- [x] `kernel/src/services/shell/commands/system.rs` -- uname 0.22.0 -> 0.23.0
- [x] `kernel/src/desktop/renderer.rs` -- welcome message 0.22.0 -> 0.23.0
- [x] `kernel/src/desktop/settings.rs` -- About panel 0.22.0 -> 0.23.0

---

## Summary

| Sprint | Focus | Files | LOC (approx) |
|--------|-------|-------|---------------|
| 10.0 | Rendering (VSync + Damage) | 8 | ~2,200 |
| 10.1 | PipeWire/PulseAudio | 11 | ~3,500 |
| 10.2 | NetworkManager | 12 | ~3,200 |
| 10.3 | BlueZ Bluetooth | 10 | ~2,600 |
| 10.4 | XWayland Enhancements | 9 | ~2,400 |
| 10.5 | Power Management | 7 | ~2,800 |
| 10.6 | KDE Features (KRunner/Baloo/Activities) | 12 | ~3,400 |
| 10.7 | USB Hotplug + V4L2 + Multi-Monitor | 10 | ~2,800 |
| 10.8 | Session Management | 8 | ~2,200 |
| 10.9 | Performance Optimization | 7 | ~1,800 |
| 10.10 | Integration + Polish | 8 | ~500 |
| **Total** | | **~95 files** | **~26,000 LOC** |
