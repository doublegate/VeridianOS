# KDE Plasma 6 Porting Guide for VeridianOS

**Version**: 1.0.0
**Date**: March 7, 2026
**Status**: Planning
**Target**: KDE Plasma 6.x on VeridianOS x86_64
**Estimated Effort**: 18-24 months, ~175 tasks across 11 sprints

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [KDE Plasma 6 Architecture](#2-kde-plasma-6-architecture)
3. [Complete Dependency Tree](#3-complete-dependency-tree)
4. [Infrastructure Gap Analysis](#4-infrastructure-gap-analysis)
5. [Porting Strategy](#5-porting-strategy)
6. [Wayland Protocol Extensions](#6-wayland-protocol-extensions)
7. [KWin Integration Architecture](#7-kwin-integration-architecture)
8. [Build Infrastructure](#8-build-infrastructure)
9. [Testing Strategy](#9-testing-strategy)
10. [Performance Considerations](#10-performance-considerations)
11. [Timeline and Effort](#11-timeline-and-effort)

---

## 1. Executive Summary

### Rationale

VeridianOS (v0.21.0) ships a custom in-kernel Wayland compositor and desktop environment with 9 built-in applications, 153 shell commands, and a GPU-accelerated renderer. While functional for embedded and lightweight use, this DE lacks the full-featured application ecosystem, hardware support, theming, accessibility, and extensibility that users expect from a modern desktop operating system.

KDE Plasma 6 is the most mature Wayland-native desktop environment available, offering:
- **KWin**: Production-grade Wayland compositor with 50+ protocol extensions, GPU compositing, and effects
- **Qt 6 + KDE Frameworks 6**: 80+ libraries providing UI toolkit, file management, system integration
- **Breeze**: Cohesive visual design with icons, themes, and fonts
- **Core applications**: Dolphin (files), Konsole (terminal), Kate (editor), Spectacle (screenshots), System Settings
- **Accessibility**: Screen reader, high contrast, keyboard navigation
- **Internationalization**: Full Unicode, CJK, RTL, and 70+ language translations

### Scope

This guide covers porting the complete KDE Plasma 6 desktop stack from the ground up, starting with dynamic linking infrastructure and culminating in a fully functional Plasma session. The existing custom DE is preserved as a lightweight fallback session type.

**In scope**: Dynamic linker, libc extensions, libstdc++, DRM/KMS user-space interface, Mesa/EGL, D-Bus, Qt 6, KDE Frameworks 6, KWin, Plasma Desktop, Breeze, core KDE applications, XWayland compatibility layer.

**Out of scope**: Flatpak/Snap packaging, Wayland-native screen recording (initially), Plasma Mobile, KDE Connect, advanced KDE PIM suite.

### Prerequisites

The following VeridianOS infrastructure must be functional before KDE porting begins:
- ELF loader with dynamic linking (`userland/ld-veridian/ld-veridian.c`)
- libc with POSIX thread support (`userland/libc/src/pthread.c`)
- Unix domain sockets with SCM_RIGHTS (`kernel/src/net/unix_socket.rs`)
- VirtIO GPU with 3D support (`kernel/src/graphics/gpu_accel.rs`)
- Process model with fork/exec, signals, CoW
- GCC 14.2 with C++ support

### Expected Outcome

A complete KDE Plasma 6 desktop session bootable in QEMU with VirtIO 3D (virgl), providing:
- KWin Wayland compositor with DRM/KMS backend
- Full Plasma Desktop shell with taskbar, system tray, application launcher
- Breeze theme (Qt + icon + cursor themes)
- Core applications: Dolphin, Konsole, Kate, Spectacle, System Settings
- D-Bus system and session buses
- XWayland for X11 application compatibility
- Session switching between built-in DE and KDE Plasma at login

---

## 2. KDE Plasma 6 Architecture

### Component Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        KDE Plasma Desktop                          │
│  ┌──────────────┐ ┌──────────┐ ┌─────────┐ ┌───────────────────┐  │
│  │ plasma-shell │ │ plasmoid │ │ Breeze  │ │ System Settings   │  │
│  │ (desktop +   │ │ widgets  │ │ theme + │ │ (KCMs)            │  │
│  │  panels)     │ │          │ │ icons   │ │                   │  │
│  └──────┬───────┘ └────┬─────┘ └────┬────┘ └────────┬──────────┘  │
│         │              │            │               │              │
│  ┌──────┴──────────────┴────────────┴───────────────┴──────────┐  │
│  │              KDE Frameworks 6 (~80 libraries)               │  │
│  │  KConfig, KCoreAddons, KI18n, KIO, Solid, KWindowSystem,   │  │
│  │  KNotifications, KXmlGui, KIconThemes, KWidgetsAddons, ...  │  │
│  └──────────────────────────┬──────────────────────────────────┘  │
│                             │                                      │
│  ┌──────────────────────────┴──────────────────────────────────┐  │
│  │                     Qt 6 Framework                          │  │
│  │  QtCore, QtGui, QtWidgets, QtQml, QtQuick, QtWayland,      │  │
│  │  QtDBus, QtNetwork, QtSvg, Qt5Compat                       │  │
│  └──────────────────────────┬──────────────────────────────────┘  │
│                             │                                      │
│  ┌────────────┐ ┌───────────┴──────────┐ ┌─────────────────────┐  │
│  │   KWin     │ │  System Libraries    │ │   D-Bus             │  │
│  │ (Wayland   │ │  Mesa/EGL, freetype, │ │  system + session   │  │
│  │ compositor)│ │  harfbuzz, fontconfig,│ │  bus                │  │
│  │            │ │  libinput, xkbcommon  │ │                     │  │
│  └──────┬─────┘ └──────────┬───────────┘ └──────────┬──────────┘  │
│         │                  │                        │              │
│  ┌──────┴──────────────────┴────────────────────────┴──────────┐  │
│  │                 VeridianOS Kernel / User-Space               │  │
│  │  DRM/KMS (/dev/dri/card0), evdev (/dev/input/event*),       │  │
│  │  Unix sockets + SCM_RIGHTS, libc, libstdc++, ld-veridian    │  │
│  └─────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

### D-Bus Dependency Map

D-Bus is the central nervous system of KDE Plasma. Nearly every component communicates through it:

```
System Bus (/var/run/dbus/system_bus_socket):
├── org.freedesktop.login1        (logind shim -- session/seat/VT management)
├── org.freedesktop.UPower        (power/battery -- stub for QEMU)
├── org.freedesktop.NetworkManager (network -- stub initially)
├── org.freedesktop.PolicyKit1    (polkit -- authorization)
└── org.freedesktop.hostname1     (hostname management)

Session Bus ($XDG_RUNTIME_DIR/bus):
├── org.kde.KWin                  (compositor control, effects, scripting)
├── org.kde.plasmashell           (desktop shell, panels, widgets)
├── org.kde.kglobalaccel          (global keyboard shortcuts)
├── org.kde.StatusNotifierWatcher  (system tray protocol)
├── org.kde.KScreen               (display configuration)
├── org.kde.Solid.PowerManagement  (power management)
├── org.freedesktop.Notifications  (desktop notifications)
├── org.freedesktop.portal.Desktop (xdg-desktop-portal)
└── org.kde.kded6                 (KDE daemon -- modules and services)
```

### Build System

KDE Plasma 6 uses **CMake** exclusively, with **Extra CMake Modules (ECM)** providing KDE-specific macros. The build dependency chain:

```
CMake 3.28+ → ECM 6.x → KDE Frameworks 6 → Plasma Desktop
                           ↓
                    Qt 6.6+ (qmake/CMake hybrid)
```

Key build system facts:
- ECM provides `KDEInstallDirs`, `KDECMakeSettings`, `KDECompilerSettings`
- Qt 6 uses CMake natively (no more qmake for building Qt itself since Qt 6.0)
- pkg-config is used extensively for C library discovery
- Meson is used for Mesa, libinput, libxkbcommon, wayland-protocols

---

## 3. Complete Dependency Tree

The dependency stack is organized into 8 layers, built bottom-up. Each library includes upstream version, build system, VeridianOS-specific patches needed, and effort estimate.

### Layer 0: OS Infrastructure

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| ld-veridian (dynamic linker) | N/A | Custom | Wire dlopen/dlsym/dlclose, test with real .so files, lazy binding, symbol versioning | 3-4 weeks |
| libc extensions | N/A | Custom | iconv, locale, getpwuid/getgrgid, mmap MAP_SHARED, atexit/__cxa_atexit, epoll | 2-3 weeks |
| libstdc++ (GCC 14.2) | 14.2 | GCC build | Port libunwind for exceptions, RTTI support, thread_local, `<filesystem>`, allocator | 3-4 weeks |
| DRM/KMS user-space interface | N/A | Custom | `/dev/dri/card0` device node, ioctl surface (VERSION, MODE_*, GEM, dumb buffers) | 3-4 weeks |
| evdev input interface | N/A | Custom | `/dev/input/event*` device nodes, ioctl (EVIOCG*), event struct (type/code/value) | 1-2 weeks |

### Layer 1: System Libraries

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| zlib | 1.3.1 | CMake/Makefile | Minimal -- configure `--host=x86_64-veridian` | 1-2 days |
| libffi | 3.4.6 | Autotools | x86_64 assembly backend, configure host triple | 2-3 days |
| pcre2 | 10.43 | CMake | CMake platform module, JIT disabled initially | 2-3 days |
| ICU | 75.1 | Autotools | Cross-compile with host ICU for data generation, threading | 1 week |
| OpenSSL | 3.3.x | Custom Perl | Platform config `VeridianOS-x86_64`, no-asm initially, threading | 1-2 weeks |
| libxml2 | 2.13.x | CMake | ICU/iconv integration, disable catalog | 2-3 days |
| libjpeg-turbo | 3.0.x | CMake | SIMD disabled initially, NASM for later optimization | 1-2 days |
| libpng | 1.6.x | CMake | Depends on zlib, minimal patches | 1-2 days |
| libtiff | 4.6.x | CMake | Depends on zlib/libjpeg-turbo | 1-2 days |
| expat | 2.6.x | CMake | Minimal | 1 day |
| double-conversion | 3.3.x | CMake | Minimal (header-only friendly) | 1 day |

### Layer 2: Graphics Foundation

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| libdrm | 2.4.120 | Meson | VeridianOS DRM ioctl wrappers, no Intel/AMD/Nouveau backends initially | 1 week |
| wayland (libraries) | 1.23.x | Meson | Unix socket transport (already supported), wayland-scanner (host tool) | 1-2 weeks |
| wayland-protocols | 1.36 | Meson | Install-only (XML protocol definitions), no compilation needed | 1-2 days |
| libepoxy | 1.5.10 | Meson | EGL/GL dispatch, VeridianOS EGL platform | 3-5 days |
| libinput | 1.26.x | Meson | evdev backend, udev stub or replacement, seat management | 1-2 weeks |
| libxkbcommon | 1.7.x | Meson | Keymap compilation, compose support, wayland integration | 1 week |

### Layer 3: Mesa / EGL / OpenGL

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| Mesa | 24.2.x | Meson | llvmpipe (CPU) + virgl (VirtIO 3D) targets, EGL wayland platform, VeridianOS DRM backend, thread primitives, mmap | 4-6 weeks |

Mesa is the single largest porting effort outside Qt. Key considerations:
- **llvmpipe**: Software rasterizer, requires LLVM (can use system LLVM from GCC build)
- **virgl**: VirtIO GPU 3D renderer, talks to host GPU via VirtIO protocol
- **EGL platform**: Needs `egl_dri2.c` platform shim for VeridianOS
- **DRI loader**: Needs `/dev/dri/renderD128` (render node) for Mesa DRI3

### Layer 4: Font / Text Stack

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| FreeType | 2.13.x | CMake/Meson | Minimal -- zlib/libpng integration | 2-3 days |
| HarfBuzz | 9.0.x | Meson | FreeType + ICU integration, threading | 3-5 days |
| Fontconfig | 2.15.x | Meson | Cache directory, font path configuration, `/etc/fonts/fonts.conf` | 1 week |
| Font packages | N/A | Install | DejaVu Sans, Noto Sans/Serif/Mono, Liberation, Noto CJK | 1-2 days |

### Layer 5: System Services

| Library | Version | Build System | Patches Needed | Effort |
|---------|---------|-------------|----------------|--------|
| D-Bus (reference impl) | 1.15.x | Meson | Unix socket transport, user/group lookup, activation, system/session bus | 3-4 weeks |
| logind shim | N/A | Custom | Session/seat/VT management via D-Bus, device access, PAM stub | 2-3 weeks |
| Polkit | 124.x | Meson | D-Bus service, authorization rules, JS engine (duktape) | 1-2 weeks |

### Layer 6: Qt 6

| Module | Version | Build System | Patches Needed | Effort |
|--------|---------|-------------|----------------|--------|
| QtBase (Core/Gui/Widgets/Network/DBus/Sql/Xml) | 6.8.x | CMake | QPA plugin for VeridianOS, event loop (epoll), threading (pthread), locale, freetype+fontconfig rendering, EGL/OpenGL integration, D-Bus client | 4-6 weeks |
| QtDeclarative (Qml/Quick) | 6.8.x | CMake | JIT disabled initially (interpreter mode), scene graph with OpenGL | 1-2 weeks |
| QtWayland | 6.8.x | CMake | libwayland-client integration, xdg-shell, xdg-decoration | 1-2 weeks |
| QtSvg | 6.8.x | CMake | Minimal, depends on QtBase | 2-3 days |
| Qt5Compat | 6.8.x | CMake | Text codecs for legacy KDE code | 2-3 days |
| QtShaderTools | 6.8.x | CMake | SPIR-V cross-compiler for Qt Quick rendering | 3-5 days |
| QtTools (linguist, etc.) | 6.8.x | CMake | Host-only build tools | 1-2 days |

**Qt Platform Abstraction (QPA)**: A custom QPA plugin (`qveridian`) is the primary integration point. It bridges Qt's platform-agnostic API to VeridianOS's Wayland client, EGL surface, and input subsystems. The QPA plugin must implement:
- `QPlatformIntegration` (screen enumeration, clipboard, drag-and-drop)
- `QPlatformWindow` (Wayland surface lifecycle)
- `QPlatformOpenGLContext` (EGL context management)
- `QPlatformInputContext` (keyboard/mouse via libinput)
- `QPlatformTheme` (font database, icons, dialogs)

### Layer 7: KDE Frameworks 6

KDE Frameworks are organized into three tiers by dependency complexity:

**Tier 1** (no KDE dependencies, only Qt):

| Framework | Purpose | Effort |
|-----------|---------|--------|
| ECM (Extra CMake Modules) | Build system macros, platform detection | 2-3 days |
| KConfig | Configuration file management | 2-3 days |
| KCoreAddons | Core utilities (jobs, plugins, text manipulation) | 2-3 days |
| KI18n | Internationalization (gettext wrapper) | 1-2 days |
| KWidgetsAddons | Additional Qt widgets | 1-2 days |
| KDBusAddons | D-Bus helper utilities | 1-2 days |
| KGuiAddons | GUI helper utilities (color, font, key sequence) | 1-2 days |
| KItemViews | Enhanced item view widgets | 1 day |
| KItemModels | Proxy models for Qt's model/view | 1 day |
| KColorScheme | Color scheme management | 1 day |
| Solid | Hardware device discovery | 3-5 days (needs device shim) |
| Sonnet | Spell checking framework | 1-2 days |
| KArchive | Archive handling (tar, zip) | 1-2 days |
| KCodecs | Text encoding/decoding | 1 day |
| KCompletion | Text completion framework | 1 day |
| ThreadWeaver | Multi-threaded job framework | 2-3 days |

**Tier 2** (depend on Tier 1):

| Framework | Purpose | Effort |
|-----------|---------|--------|
| KNotifications | Desktop notification system | 2-3 days |
| KXmlGui | XML-based GUI building | 1-2 days |
| KIconThemes | Icon theme engine (freedesktop.org spec) | 2-3 days |
| KConfigWidgets | Widgets for KConfig | 1-2 days |
| KXMLRPC | XML-RPC client | 1 day |
| KGlobalAccel | Global keyboard shortcuts | 2-3 days |
| KCrash | Crash handler (Dr. Konqi) | 1-2 days |
| KAuth | Authorization framework (Polkit backend) | 2-3 days |
| KJobWidgets | Widgets for KJob progress | 1 day |
| KBookmarks | Bookmark management | 1-2 days |

**Tier 3** (depend on Tier 1+2, may have external dependencies):

| Framework | Purpose | Effort |
|-----------|---------|--------|
| KIO | Virtual filesystem, network-transparent I/O | 1-2 weeks |
| KWindowSystem | Window management (Wayland backend) | 1 week |
| KNewStuff | Content download framework | 2-3 days |
| KService | Service/plugin discovery | 2-3 days |
| KParts | Document component framework | 2-3 days |
| KTextWidgets | Rich text editing | 1-2 days |
| KWallet | Credential storage | 3-5 days |
| KDeclarative | QML integration for KDE | 2-3 days |
| Plasma Framework | Plasma shell library (containments, applets, DataEngine) | 1-2 weeks |
| KPackage | Package format for Plasma add-ons | 2-3 days |
| KActivities | Activity management (virtual desktops++) | 2-3 days |

### Layer 8: KWin + Plasma Desktop

| Component | Purpose | Effort |
|-----------|---------|--------|
| KWin | Wayland compositor (DRM/KMS, effects, scripting) | 4-6 weeks |
| kdecoration | Window decoration framework | 1 week |
| plasma-workspace | Session management, lock screen, logout | 2-3 weeks |
| plasma-desktop | Desktop containment, panel, app launcher | 2-3 weeks |
| Breeze (theme) | Qt style, window decorations, icon theme, cursor theme | 1-2 weeks |
| KScreen | Display configuration | 1 week |
| PowerDevil | Power management | 1 week |
| Plasma System Monitor | System resource monitoring | 1 week |

### Layer 8+: Core Applications

| Application | Purpose | Effort |
|-------------|---------|--------|
| Dolphin | File manager | 1-2 weeks |
| Konsole | Terminal emulator | 1-2 weeks |
| Kate | Text editor | 1-2 weeks |
| Spectacle | Screenshot utility | 3-5 days |
| Gwenview | Image viewer | 1 week |
| Ark | Archive manager | 3-5 days |
| KCalc | Calculator | 2-3 days |

---

## 4. Infrastructure Gap Analysis

### Gap 1: Dynamic Linking Not Functional

**Current state**: `userland/ld-veridian/ld-veridian.c` (1800+ lines) implements PLT/GOT relocation, TLS support, and `LD_LIBRARY_PATH`. However, it has never been tested with real shared objects. `dlopen`/`dlsym`/`dlclose` in `userland/libc/src/posix_stubs.c:596-616` return `NULL`/`-1` unconditionally.

**Required**: Full dynamic linking with lazy binding, symbol versioning (GNU hash), `RTLD_GLOBAL`/`RTLD_LOCAL`/`RTLD_NOW`/`RTLD_LAZY` flags, `dladdr()`, `dl_iterate_phdr()`, init/fini arrays, `DT_NEEDED` recursive loading.

**Effort**: 3-4 weeks
**Risk**: HIGH -- this blocks everything. Qt 6 plugins, KDE Frameworks plugins, Mesa drivers, and D-Bus activation all require dynamic linking.
**Dependencies**: None (foundational)

### Gap 2: No D-Bus

**Current state**: No D-Bus implementation exists. KDE uses D-Bus for virtually all inter-component communication: KWin control, Plasma shell coordination, notifications, global shortcuts, power management, system tray, device hotplug.

**Required**: Full `dbus-1` reference implementation with system bus (`/var/run/dbus/system_bus_socket`) and session bus (`$XDG_RUNTIME_DIR/bus`), activation (launching services on demand), signal matching, property interface, introspection.

**Effort**: 3-4 weeks
**Risk**: HIGH -- without D-Bus, KDE components cannot communicate.
**Dependencies**: Unix domain sockets (have), dynamic linking (Gap 1), libc extensions

### Gap 3: No libstdc++ / C++ Runtime

**Current state**: GCC 14.2 is built for VeridianOS but the C++ standard library has never been ported. No exception handling (libunwind/libgcc_s), no RTTI, no STL containers in user-space, no `<thread>`, `<mutex>`, `<filesystem>`.

**Required**: Full libstdc++ with exception handling (DWARF unwinder via libgcc_s or LLVM libunwind), RTTI, `<thread>` backed by pthread, `<filesystem>` backed by POSIX I/O, `<chrono>` backed by `clock_gettime`.

**Effort**: 3-4 weeks
**Risk**: HIGH -- Qt 6 and all KDE code is C++. Exceptions are used pervasively.
**Dependencies**: libc extensions, dynamic linking (Gap 1)

### Gap 4: Wayland Transport Mismatch

**Current state**: The kernel's Wayland compositor (`kernel/src/desktop/wayland/`) uses custom syscalls (`SYS_WL_CONNECT=240`, `SYS_WL_SEND=241`, `SYS_WL_RECV=242`) for client-server communication. The user-space client (`userland/libwayland/wayland-client.c`) uses these syscalls directly. Standard `libwayland-client` expects Unix domain socket transport with `sendmsg`/`recvmsg` and `SCM_RIGHTS` for fd passing.

**Required**: KWin uses standard `libwayland-server` with Unix domain sockets. This is compatible with VeridianOS's existing Unix socket + SCM_RIGHTS support (`kernel/src/net/unix_socket.rs`). The kernel syscall-based Wayland compositor remains for the built-in DE. KWin runs its own independent Wayland server in user-space.

**Effort**: Minimal for KWin (it brings its own libwayland). 1-2 weeks for porting libwayland itself.
**Risk**: LOW -- well-understood standard transport.
**Dependencies**: Unix domain sockets (have), SCM_RIGHTS (have)

### Gap 5: No User-Space DRM/KMS Interface

**Current state**: `kernel/src/graphics/gpu_accel.rs` implements VirtIO GPU 3D protocol, GEM buffer management, and DRM KMS structures in-kernel, but there are no `/dev/dri/card0` or `/dev/dri/renderD128` device nodes. No user-space ioctl interface exists.

**Required**: DRM device nodes with ioctl interface:
- `DRM_IOCTL_VERSION` -- driver name/version
- `DRM_IOCTL_MODE_GETRESOURCES` -- connectors, CRTCs, encoders
- `DRM_IOCTL_MODE_SETCRTC` -- set display mode
- `DRM_IOCTL_MODE_PAGE_FLIP` -- vsync-aware page flip
- `DRM_IOCTL_MODE_CREATE_DUMB` / `MAP_DUMB` / `DESTROY_DUMB` -- dumb buffer allocation
- `DRM_IOCTL_GEM_CLOSE` -- GEM handle management
- `DRM_IOCTL_PRIME_HANDLE_TO_FD` / `FD_TO_HANDLE` -- DMA-BUF sharing

**Effort**: 3-4 weeks
**Risk**: HIGH -- KWin's DRM backend requires this for display output.
**Dependencies**: Device node infrastructure, mmap for buffer mapping

### Gap 6: No Mesa / EGL

**Current state**: `kernel/src/graphics/gpu_accel.rs` has an OpenGL ES 2.0 software rasterizer using integer/fixed-point math, but this is kernel-side only. No user-space OpenGL or EGL implementation exists.

**Required**: Mesa with:
- **llvmpipe** driver (software rasterizer for baseline functionality)
- **virgl** driver (VirtIO GPU 3D for hardware-accelerated rendering in QEMU)
- **EGL wayland platform** (for Qt/KWin to create GL contexts on Wayland surfaces)
- **GBM** (Generic Buffer Management for KWin's DRM backend)

**Effort**: 4-6 weeks
**Risk**: HIGH -- Qt 6 Quick and KWin effects require OpenGL.
**Dependencies**: DRM/KMS interface (Gap 5), dynamic linking (Gap 1), libdrm, LLVM

### Gap 7: No FreeType / Fontconfig / HarfBuzz

**Current state**: The kernel's built-in DE uses a bitmap font renderer (`kernel/src/desktop/renderer.rs`). No scalable font support exists in user-space.

**Required**: Full text rendering stack:
- **FreeType**: TrueType/OpenType rasterization
- **HarfBuzz**: Complex text shaping (ligatures, kerning, CJK, Arabic, Devanagari)
- **Fontconfig**: Font discovery, matching, configuration
- **System fonts**: DejaVu, Noto (including CJK), Liberation font families

**Effort**: 2-3 weeks total
**Risk**: MEDIUM -- well-established portable libraries, but fontconfig's filesystem scanning needs testing.
**Dependencies**: zlib, libpng, ICU

### Gap 8: No libinput / libxkbcommon

**Current state**: Keyboard input is handled via PS/2 polling (ports 0x60/0x64) and serial. The kernel routes input events to the compositor directly. No user-space evdev or libinput infrastructure exists.

**Required**:
- **evdev interface**: `/dev/input/event*` device nodes with `struct input_event` and `EVIOCG*` ioctls
- **libinput**: User-space input handling library (pointer acceleration, tap-to-click, gesture recognition)
- **libxkbcommon**: Keymap compilation and state tracking (XKB format)

**Effort**: 2-3 weeks total
**Risk**: MEDIUM -- libinput needs udev or a shim for device discovery.
**Dependencies**: evdev device nodes, libc

### Gap 9: No logind / Session Management

**Current state**: No session or seat management. The built-in DE starts directly from the init system without session tracking.

**Required**: KWin needs a logind-compatible D-Bus interface for:
- Session registration (session ID, seat, VT)
- Device access (`TakeDevice`/`ReleaseDevice` for `/dev/dri/*`, `/dev/input/*`)
- VT switching
- Idle/sleep/shutdown inhibition
- `Activate`/`Lock`/`Unlock` session signals

**Effort**: 2-3 weeks
**Risk**: MEDIUM -- can implement a minimal shim that only supports the APIs KWin actually uses, rather than full systemd-logind.
**Dependencies**: D-Bus (Gap 2), device node infrastructure

### Gap 10: No System Font Packages

**Current state**: The kernel uses hardcoded bitmap fonts. No TrueType/OpenType font files are installed on the rootfs.

**Required**:
- DejaVu Sans / Sans Mono / Serif (default UI font family)
- Noto Sans / Serif / Mono (Unicode coverage)
- Noto Sans CJK (Chinese/Japanese/Korean)
- Liberation Sans / Serif / Mono (metric-compatible with Arial/Times/Courier)
- Noto Color Emoji (optional)
- Fontconfig configuration (`/etc/fonts/fonts.conf`, `/etc/fonts/conf.d/`)

**Effort**: 1-2 days (package existing font files into rootfs)
**Risk**: LOW
**Dependencies**: Fontconfig (Gap 7), BlockFS or ext4 rootfs with enough space (~200MB for full font set)

### Dependency Graph

```
Gap 1 (Dynamic Linking) ──→ Gap 2 (D-Bus) ──→ Gap 9 (logind)
         │                       │                    │
         │                       ↓                    ↓
         ├──→ Gap 3 (libstdc++) ──→ Qt 6 ──→ KDE Frameworks ──→ Plasma
         │                          ↑              ↑
         │                          │              │
         ├──→ Gap 5 (DRM/KMS) ──→ Gap 6 (Mesa) ──→ KWin
         │         ↑
         │         │
         ├──→ Gap 8 (libinput) ──→ KWin
         │
         ├──→ Gap 7 (fonts) ──→ Qt 6
         │
         └──→ Gap 4 (Wayland transport) -- minimal, KWin uses standard libwayland
```

---

## 5. Porting Strategy

### Build Order

Libraries are built in strict dependency order. Each layer is fully validated before proceeding to the next.

#### Phase A: Foundation (Sprints 9.0-9.1)

```
1. ld-veridian (test with trivial .so)
2. libc extensions (iconv, locale, mmap MAP_SHARED, epoll)
3. libgcc_s / libunwind (exception unwinding)
4. libstdc++ (full C++ runtime)
5. DRM/KMS device nodes + ioctl interface
6. evdev device nodes + ioctl interface
```

#### Phase B: System Libraries (Sprints 9.2-9.4)

```
7.  zlib
8.  expat
9.  libffi
10. double-conversion
11. pcre2
12. ICU
13. OpenSSL (or libressl)
14. libxml2
15. libjpeg-turbo
16. libpng
17. libtiff
18. FreeType
19. HarfBuzz (depends on FreeType, ICU)
20. Fontconfig (depends on FreeType, expat)
21. Font packages
22. libxkbcommon
```

#### Phase C: Graphics + IPC (Sprints 9.3, 9.5)

```
23. libdrm (depends on DRM/KMS device nodes)
24. wayland + wayland-protocols
25. LLVM (for Mesa llvmpipe -- may reuse GCC build's LLVM)
26. Mesa (llvmpipe + virgl, EGL, GBM)
27. libepoxy
28. libinput
29. D-Bus
30. logind shim
31. Polkit
```

#### Phase D: Qt 6 (Sprint 9.6)

```
32. Qt 6 host tools (moc, rcc, uic, qsb -- build for host first)
33. QtBase cross-compile (Core, Gui, Widgets, Network, DBus, Sql, Xml)
34. QPA plugin (qveridian)
35. QtShaderTools
36. QtDeclarative (Qml, Quick)
37. QtWayland
38. QtSvg
39. Qt5Compat
```

#### Phase E: KDE Stack (Sprints 9.7-9.10)

```
40. ECM
41. KDE Frameworks Tier 1 (16 libraries)
42. KDE Frameworks Tier 2 (10 libraries)
43. KDE Frameworks Tier 3 (11 libraries)
44. KWin
45. kdecoration + Breeze
46. plasma-workspace
47. plasma-desktop
48. Core apps (Dolphin, Konsole, Kate, Spectacle)
49. XWayland (for X11 app compatibility)
50. Integration testing + boot sequence
```

### Configure Flags and Patch Patterns

#### CMake Platform Module

Create `Platform/VeridianOS.cmake` for CMake's platform detection:

```cmake
set(VERIDIAN 1)
set(UNIX 1)

# VeridianOS is POSIX-like but not Linux
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_DL_LIBS "dl")
set(CMAKE_SHARED_LIBRARY_PREFIX "lib")
set(CMAKE_SHARED_LIBRARY_SUFFIX ".so")
set(CMAKE_SHARED_MODULE_PREFIX "lib")
set(CMAKE_SHARED_MODULE_SUFFIX ".so")
set(CMAKE_EXECUTABLE_FORMAT "ELF")

# Thread support
set(CMAKE_USE_PTHREADS_INIT 1)
set(CMAKE_THREAD_LIBS_INIT "-lpthread")
set(Threads_FOUND TRUE)
set(CMAKE_HAVE_THREADS_LIBRARY 1)

# No /proc filesystem
set(CMAKE_SYSTEM_HAS_PROC FALSE)
```

#### CMake Cross-Compilation Toolchain

Extend the existing generator in `kernel/src/pkg/sdk/toolchain.rs:480-541`:

```cmake
# VeridianToolchain.cmake (cross-compilation)
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(CMAKE_SYSROOT /opt/veridian-sysroot)

set(CMAKE_C_COMPILER x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)
set(CMAKE_AR x86_64-veridian-ar)
set(CMAKE_RANLIB x86_64-veridian-ranlib)
set(CMAKE_STRIP x86_64-veridian-strip)

set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# pkg-config for cross-compilation
set(ENV{PKG_CONFIG_PATH} "${CMAKE_SYSROOT}/usr/lib/pkgconfig:${CMAKE_SYSROOT}/usr/share/pkgconfig")
set(ENV{PKG_CONFIG_SYSROOT_DIR} "${CMAKE_SYSROOT}")
```

#### Qt 6 mkspec

Create `mkspecs/veridian-g++/qmake.conf`:

```makefile
# Qt 6 platform specification for VeridianOS
include(../common/linux.conf)   # Start with Linux as baseline
include(../common/gcc-base-unix.conf)
include(../common/g++-unix.conf)

QMAKE_PLATFORM = veridian

# Override Linux-specific settings
QMAKE_CFLAGS += -D__veridian__
QMAKE_CXXFLAGS += -D__veridian__
QMAKE_LFLAGS += -Wl,--dynamic-linker=/lib/ld-veridian.so.1

# No Linux-specific features
QMAKE_LIBS_THREAD = -lpthread
QMAKE_INCDIR =
QMAKE_LIBDIR =

load(qt_config)
```

For CMake-based Qt 6 build (preferred), add `cmake/platforms/VeridianOS.cmake`:

```cmake
# Qt 6 platform configuration for VeridianOS
set(QT_DEFAULT_MKSPEC "veridian-g++")
set(QT_QPA_DEFAULT_PLATFORM "veridian")

# Feature flags
qt_configure_add_summary_entry(ARGS "VeridianOS" TYPE bool VALUE ON)
set(FEATURE_epoll ON)        # If epoll is implemented
set(FEATURE_inotify OFF)     # Unless implemented
set(FEATURE_linuxfb OFF)     # No Linux framebuffer
set(FEATURE_evdev ON)        # If evdev interface exists
set(FEATURE_libinput ON)
set(FEATURE_xkbcommon ON)

# Wayland and OpenGL
set(FEATURE_opengl ON)
set(FEATURE_opengles2 ON)
set(FEATURE_egl ON)
set(FEATURE_wayland ON)
```

#### ECM VeridianOS Detection

Patch `modules/ECMFindModuleHelpers.cmake` or create `find-modules/FindVeridianOS.cmake`:

```cmake
# ECM VeridianOS platform detection
if(CMAKE_SYSTEM_NAME STREQUAL "VeridianOS")
    set(VERIDIAN_OS TRUE)
    # Adjust paths, disable Linux-specific modules
    set(KDE_INSTALL_USE_QT_SYS_PATHS ON)
endif()
```

### Meson Cross-File

For Meson-based projects (Mesa, libinput, wayland, fontconfig):

```ini
# veridian-cross.ini
[binaries]
c = 'x86_64-veridian-gcc'
cpp = 'x86_64-veridian-g++'
ar = 'x86_64-veridian-ar'
strip = 'x86_64-veridian-strip'
pkg-config = 'x86_64-veridian-pkg-config'

[host_machine]
system = 'veridian'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'

[properties]
sys_root = '/opt/veridian-sysroot'
pkg_config_libdir = '/opt/veridian-sysroot/usr/lib/pkgconfig'
needs_exe_wrapper = true
```

---

## 6. Wayland Protocol Extensions

### Protocols Already Implemented (Kernel Compositor)

The kernel's built-in compositor (`kernel/src/desktop/wayland/`) implements:

| File | Protocol | Status |
|------|----------|--------|
| `compositor.rs` | `wl_compositor` (v5) | Implemented (surface + region management) |
| `surface.rs` | `wl_surface` | Implemented (commit, damage, buffer attach) |
| `buffer.rs` | `wl_shm` + `wl_buffer` | Implemented (SHM pool, format negotiation) |
| `shell.rs` | `xdg_shell` (xdg_wm_base, xdg_surface, xdg_toplevel, xdg_popup) | Implemented |
| `layer_shell.rs` | `zwlr_layer_shell_v1` | Implemented (panels, overlays) |
| `dmabuf.rs` | `zwp_linux_dmabuf_v1` | Implemented (DMA-BUF import) |
| `idle_inhibit.rs` | `zwp_idle_inhibit_manager_v1` | Implemented |
| `output.rs` | `wl_output` | Implemented (mode, geometry, scale) |
| `protocol.rs` | Wire protocol parsing | Implemented (message header, argument types) |

These are available only for the built-in DE (kernel syscall transport). KWin implements its own Wayland server and brings all protocols it needs.

### KDE-Specific Protocols (Implemented by KWin)

KWin implements approximately 50 Wayland protocols. The following are KDE-specific extensions that the Plasma shell depends on:

| Protocol | Purpose | Used By |
|----------|---------|---------|
| `org_kde_plasma_shell` | Desktop/panel surface roles, auto-hide, skip taskbar | plasma-shell |
| `org_kde_plasma_window_management` | Window list, minimize/maximize/close control | taskbar, app switcher |
| `org_kde_kwin_server_decoration_manager` | Server-side vs client-side decorations negotiation | all windows |
| `org_kde_kwin_blur_manager` | Blur effect behind translucent surfaces | Plasma panels, Dolphin |
| `org_kde_kwin_contrast_manager` | Contrast adjustment behind surfaces | Plasma panels |
| `org_kde_kwin_slide_manager` | Slide animation for panels/popups | Plasma panels |
| `org_kde_kwin_dpms` | Display power management | PowerDevil |
| `org_kde_kwin_outputdevice` | Extended output configuration | KScreen |
| `org_kde_kwin_outputmanagement` | Output configuration changes | KScreen |
| `org_kde_kwin_idle` | Idle detection for screen locking | kscreenlocker |

### Standard Protocols Required by KWin

| Protocol | Specification | Purpose |
|----------|--------------|---------|
| `xdg-shell` (v6) | `xdg-shell.xml` | Window management (toplevel, popup) |
| `xdg-decoration-unstable-v1` | `xdg-decoration.xml` | Server-side decoration negotiation |
| `xdg-output-unstable-v1` | `xdg-output.xml` | Logical output geometry |
| `xdg-activation-v1` | `xdg-activation.xml` | Window activation tokens |
| `wlr-layer-shell-unstable-v1` | `wlr-layer-shell.xml` | Panel/overlay layers |
| `wp-presentation-time` | `presentation-time.xml` | Frame timing feedback |
| `wp-viewporter` | `viewporter.xml` | Surface scaling/cropping |
| `wp-fractional-scale-v1` | `fractional-scale.xml` | Fractional display scaling |
| `wp-cursor-shape-v1` | `cursor-shape.xml` | Cursor shape protocol |
| `wp-content-type-v1` | `content-type.xml` | Content type hints |
| `wp-linux-drm-syncobj-v1` | `drm-syncobj.xml` | Explicit sync for GPU |
| `ext-session-lock-v1` | `ext-session-lock.xml` | Screen locking |
| `ext-idle-notify-v1` | `ext-idle-notify.xml` | Idle state notification |
| `wp-security-context-v1` | `security-context.xml` | Sandboxed client security |
| `zwp-input-method-v2` | `input-method.xml` | Virtual keyboard / input methods |
| `zwp-text-input-v3` | `text-input.xml` | Text input protocol |
| `zwp-pointer-constraints-v1` | `pointer-constraints.xml` | Pointer lock/confine |
| `zwp-relative-pointer-v1` | `relative-pointer.xml` | Relative pointer motion |
| `zwp-tablet-v2` | `tablet.xml` | Tablet/stylus input |
| `xdg-foreign-v2` | `xdg-foreign.xml` | Cross-client surface embedding |

All of these are implemented by KWin itself, not the OS. VeridianOS only needs to provide the transport (Unix domain sockets + SCM_RIGHTS).

---

## 7. KWin Integration Architecture

### Overview

KWin runs as a user-space Wayland compositor, replacing the kernel's built-in compositor for KDE sessions. This aligns with VeridianOS's microkernel philosophy of keeping functionality in user-space.

```
┌─────────────────────────────────────────────────────┐
│                    KWin Process                      │
│                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────┐  │
│  │ Wayland      │  │ OpenGL       │  │ Effects   │  │
│  │ Server       │  │ Compositor   │  │ System    │  │
│  │ (libwayland) │  │ (EGL+GLES)  │  │ (plugins) │  │
│  └──────┬───────┘  └──────┬───────┘  └─────┬─────┘  │
│         │                 │                │          │
│  ┌──────┴─────────────────┴────────────────┴──────┐  │
│  │              KWin Platform Backend              │  │
│  │  ┌─────────────┐  ┌───────────┐  ┌──────────┐  │  │
│  │  │ DRM/KMS     │  │ libinput  │  │ logind   │  │  │
│  │  │ Backend     │  │ Input     │  │ Session  │  │  │
│  │  │ (page flip, │  │ (pointer, │  │ (device  │  │  │
│  │  │  mode set)  │  │  keyboard)│  │  access) │  │  │
│  │  └──────┬──────┘  └─────┬─────┘  └────┬─────┘  │  │
│  └─────────┼───────────────┼──────────────┼────────┘  │
│            │               │              │           │
└────────────┼───────────────┼──────────────┼───────────┘
             │               │              │
     ┌───────┴───────┐ ┌────┴─────┐ ┌──────┴──────┐
     │ /dev/dri/card0│ │/dev/input│ │ D-Bus       │
     │ DRM ioctls    │ │ evdev    │ │ (logind API)│
     └───────────────┘ └──────────┘ └─────────────┘
             │               │              │
     ┌───────┴───────────────┴──────────────┴──────┐
     │          VeridianOS Kernel                   │
     │  gpu_accel.rs    input     unix_socket.rs    │
     └─────────────────────────────────────────────┘
```

### DRM/KMS Backend

KWin's DRM backend (`src/backends/drm/`) is the primary output backend. It requires:

1. **Device open via logind**: KWin calls `TakeDevice("/dev/dri/card0")` on D-Bus → logind shim opens the device with appropriate capabilities and passes the fd back
2. **Mode setting**: `drmModeGetResources()` → enumerate connectors → `drmModeGetConnector()` → find preferred mode → `drmModeSetCrtc()`
3. **Page flipping**: `drmModePageFlip()` with `DRM_MODE_PAGE_FLIP_EVENT` → `drmHandleEvent()` on vblank
4. **Buffer allocation**: GBM (`gbm_surface_create()`, `gbm_bo_get_fd()`) for EGL rendering targets
5. **Atomic modesetting** (optional but preferred): `DRM_IOCTL_MODE_ATOMIC` for multi-plane updates

### libinput Integration

KWin uses libinput for all input:

1. **Seat management**: libinput opens a seat, discovers input devices via udev (or shim)
2. **Event loop**: KWin polls `libinput_get_fd()` in its event loop
3. **Event dispatch**: Pointer motion, button, scroll, keyboard key, touch, tablet events
4. **Configuration**: Pointer acceleration, tap-to-click, scroll method, natural scrolling

### logind Shim

The logind shim provides a minimal D-Bus service implementing `org.freedesktop.login1`:

```
Interface: org.freedesktop.login1.Manager
  Methods:
    GetSession(session_id) → object_path
    GetSeat(seat_id) → object_path

Interface: org.freedesktop.login1.Session
  Methods:
    TakeDevice(major, minor) → fd, inactive
    ReleaseDevice(major, minor)
    TakeControl(force)
    ReleaseControl()
    Activate()
    SetIdleHint(idle)
  Signals:
    PauseDevice(major, minor, type)
    ResumeDevice(major, minor, fd)
  Properties:
    Active (bool)
    Id (string)
    Seat (object_path)
    Type (string: "wayland")
    State (string: "active")
    VTNr (uint32)
```

### Capability Mapping

VeridianOS's capability-based security integrates with KWin through the logind shim:

| Operation | Linux Mechanism | VeridianOS Mechanism |
|-----------|----------------|---------------------|
| Open DRM device | `TakeDevice` → file fd | logind shim requests GPU capability from kernel, returns fd |
| Open input device | `TakeDevice` → file fd | logind shim requests input capability, returns fd |
| Set DRM master | `DRM_IOCTL_SET_MASTER` | Granted automatically to session compositor |
| mmap framebuffer | `mmap(fd, offset)` | Capability-controlled MMIO mapping |
| VT switch | VT ioctl | Session activate/deactivate signals |

---

## 8. Build Infrastructure

### Sysroot Population

The cross-compilation sysroot (`/opt/veridian-sysroot/`) accumulates headers and libraries as each layer is built:

```
/opt/veridian-sysroot/
├── etc/
│   ├── fonts/
│   │   ├── fonts.conf
│   │   └── conf.d/
│   └── dbus-1/
│       ├── system.conf
│       └── session.conf
├── lib/
│   ├── ld-veridian.so.1
│   ├── libc.so
│   └── libstdc++.so.6
├── usr/
│   ├── include/
│   │   ├── dbus-1.0/
│   │   ├── EGL/
│   │   ├── GL/
│   │   ├── GLES2/
│   │   ├── KF6/
│   │   ├── Qt6/
│   │   ├── wayland-client.h
│   │   ├── xkbcommon/
│   │   ├── freetype2/
│   │   ├── fontconfig/
│   │   └── ...
│   ├── lib/
│   │   ├── pkgconfig/           # .pc files for all libraries
│   │   ├── cmake/               # CMake config files
│   │   │   ├── Qt6/
│   │   │   ├── KF6/
│   │   │   └── ECM/
│   │   ├── qt6/
│   │   │   └── plugins/
│   │   │       ├── platforms/
│   │   │       │   └── libqveridian.so    # QPA plugin
│   │   │       └── wayland-shell-integration/
│   │   ├── libQt6Core.so.6
│   │   ├── libKF6CoreAddons.so.6
│   │   ├── libwayland-client.so
│   │   ├── libEGL.so
│   │   ├── libGLESv2.so
│   │   ├── libdrm.so
│   │   ├── libinput.so
│   │   ├── libfontconfig.so
│   │   ├── libfreetype.so
│   │   ├── libharfbuzz.so
│   │   └── ...
│   └── share/
│       ├── fonts/
│       │   ├── dejavu/
│       │   ├── noto/
│       │   └── liberation/
│       ├── icons/
│       │   └── breeze/
│       ├── plasma/
│       ├── wayland-protocols/
│       ├── locale/
│       └── dbus-1/
│           ├── system-services/
│           └── services/
└── var/
    └── run/
        └── dbus/
```

### pkg-config Integration

Every library installs a `.pc` file. Cross-compilation wrapper script:

```bash
#!/bin/sh
# x86_64-veridian-pkg-config
export PKG_CONFIG_SYSROOT_DIR="/opt/veridian-sysroot"
export PKG_CONFIG_LIBDIR="/opt/veridian-sysroot/usr/lib/pkgconfig"
export PKG_CONFIG_PATH=""
exec pkg-config "$@"
```

### CI Pipeline

The CI pipeline extends VeridianOS's existing GitHub Actions workflow:

```yaml
# .github/workflows/kde-plasma-build.yml
jobs:
  layer-0-foundation:
    runs-on: ubuntu-latest
    steps:
      - name: Build ld-veridian + libc extensions + libstdc++
      - name: Build DRM/KMS + evdev interfaces
      - name: Archive sysroot layer 0

  layer-1-syslibs:
    needs: layer-0-foundation
    steps:
      - name: Build zlib, libffi, pcre2, ICU, OpenSSL, libxml2, image libs
      - name: Archive sysroot layer 1

  layer-2-graphics:
    needs: [layer-0-foundation, layer-1-syslibs]
    steps:
      - name: Build libdrm, wayland, Mesa (llvmpipe+virgl), libinput, xkbcommon
      - name: Archive sysroot layer 2

  layer-3-fonts:
    needs: layer-1-syslibs
    steps:
      - name: Build FreeType, HarfBuzz, Fontconfig
      - name: Install font packages
      - name: Archive sysroot layer 3

  layer-4-services:
    needs: [layer-0-foundation, layer-1-syslibs]
    steps:
      - name: Build D-Bus, logind shim, Polkit
      - name: Archive sysroot layer 4

  layer-5-qt6:
    needs: [layer-2-graphics, layer-3-fonts, layer-4-services]
    steps:
      - name: Build Qt 6 host tools
      - name: Cross-compile Qt 6 modules
      - name: Archive sysroot layer 5

  layer-6-kde:
    needs: layer-5-qt6
    steps:
      - name: Build ECM
      - name: Build KDE Frameworks Tier 1/2/3
      - name: Build KWin
      - name: Build Plasma Desktop
      - name: Archive sysroot layer 6

  integration-test:
    needs: layer-6-kde
    steps:
      - name: Build QEMU disk image with KDE
      - name: Boot test (KWin starts, Plasma shell renders)
      - name: Screenshot comparison
```

### Build Times (Estimated)

| Layer | Component | Estimated Time (16-core) |
|-------|-----------|-------------------------|
| 0 | ld-veridian + libc + libstdc++ | 30 min |
| 1 | System libraries (11 libs) | 45 min |
| 2 | Graphics (Mesa alone: 20 min) | 35 min |
| 3 | Fonts (FreeType + HarfBuzz + Fontconfig) | 10 min |
| 4 | D-Bus + logind + Polkit | 15 min |
| 5 | Qt 6 (all modules) | 60-90 min |
| 6 | KDE Frameworks (80+ libs) | 45-60 min |
| 7 | KWin + Plasma Desktop | 30-45 min |
| **Total** | | **~4-5 hours** |

---

## 9. Testing Strategy

### Layered Testing

Each layer is independently validated before proceeding:

#### Layer 0: Foundation

```bash
# Dynamic linker test
echo 'int foo(void) { return 42; }' > libtest.c
x86_64-veridian-gcc -shared -o libtest.so libtest.c
echo '#include <dlfcn.h>
int main() {
    void *h = dlopen("./libtest.so", RTLD_NOW);
    int (*f)(void) = dlsym(h, "foo");
    return f() == 42 ? 0 : 1;
}' > test_dl.c
x86_64-veridian-gcc -o test_dl test_dl.c -ldl
# Run in QEMU -- must exit 0

# libstdc++ test
echo '#include <vector>
#include <string>
#include <stdexcept>
int main() {
    try {
        std::vector<std::string> v = {"hello", "world"};
        if (v.at(5) == "x") return 1;  // throws out_of_range
    } catch (const std::out_of_range&) {
        return 0;
    }
    return 1;
}' > test_cpp.cpp
x86_64-veridian-g++ -o test_cpp test_cpp.cpp
# Run in QEMU -- must exit 0 (exception caught)

# DRM test
# Verify /dev/dri/card0 exists and responds to DRM_IOCTL_VERSION
```

#### Layer 2-4: Libraries

```bash
# Mesa test -- eglinfo or custom EGL test
# FreeType test -- render a glyph to bitmap
# D-Bus test -- dbus-send/dbus-monitor basic operation
```

#### Layer 5: Qt 6

```bash
# Build and run Qt 6 test suite (subset)
# Verify QPA plugin loads: QT_QPA_PLATFORM=veridian ./test_app
# Check: window creation, text rendering, OpenGL context, D-Bus connection
```

#### Layer 6-8: KDE

```bash
# KWin standalone test (without Plasma shell)
# Verify: KWin starts, creates Wayland socket, accepts client connections
# Plasma session boot test
# Verify: login → D-Bus → KWin → Plasma shell → desktop rendered
```

### QEMU Test Configuration

```bash
# KDE Plasma test QEMU command
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-kde.img \
    -device ide-hd,drive=disk0 \
    -m 2048M \
    -smp 4 \
    -device virtio-gpu-gl-pci \
    -display gtk,gl=on \
    -device virtio-keyboard-pci \
    -device virtio-mouse-pci \
    -serial stdio \
    -qmp unix:/tmp/qmp.sock,server,nowait
```

Key differences from current QEMU config:
- **`-m 2048M`** (up from 256M) -- KDE Plasma needs ~800MB minimum
- **`-smp 4`** -- KWin benefits from multi-core
- **`virtio-gpu-gl-pci`** -- 3D acceleration via virgl (host GPU passthrough to guest Mesa)
- **`-display gtk,gl=on`** -- enables virgl rendering on host side
- **`virtio-keyboard-pci` + `virtio-mouse-pci`** -- evdev-compatible virtio input devices (not PS/2)

### Screenshot Comparison

Use QEMU QMP for automated screenshot capture (per project conventions):

```bash
# Capture screenshot after boot
echo '{"execute":"qmp_capabilities"}{"execute":"screendump","arguments":{"filename":"/tmp/kde-boot.ppm"}}' \
    | socat - UNIX-CONNECT:/tmp/qmp.sock

# Compare against reference image (perceptual hash or SSIM)
python3 compare_screenshots.py /tmp/kde-boot.ppm reference/kde-desktop.ppm --threshold 0.95
```

### Regression Testing

All 4,095 existing kernel tests must continue passing. The KDE port adds no kernel code changes -- it is entirely user-space.

---

## 10. Performance Considerations

### Memory Budget

| Component | Estimated RAM | Notes |
|-----------|-------------|-------|
| VeridianOS kernel | ~50 MB | Kernel heap (currently 512MB static, but actual usage ~50MB) |
| D-Bus (system + session) | ~10 MB | |
| KWin | ~80-150 MB | Compositor buffers, OpenGL context, effect textures |
| Plasma shell | ~100-150 MB | QML engine, plasmoids, icon cache |
| Qt 6 runtime | ~50-80 MB | Shared across all Qt apps |
| Mesa (llvmpipe) | ~50-100 MB | Shader compilation cache, render buffers |
| Fontconfig cache | ~20-30 MB | Font enumeration cache |
| Core apps (Dolphin + Konsole) | ~50-100 MB each | |
| **Total** | **~500-800 MB** | |

**Kernel heap adjustment**: The current 512MB static kernel heap (`HEAP_MEMORY` in kernel BSS) is insufficient if user-space needs 800MB+ on top. Options:
1. Reduce kernel heap to 256MB (sufficient for kernel operations) and give the rest to user-space
2. Implement demand-paged user-space memory (currently pre-allocated)
3. Require `-m 2048M` or higher for KDE sessions

### GPU Performance

| Backend | Expected Performance | Use Case |
|---------|---------------------|----------|
| llvmpipe (CPU) | 5-15 FPS for full desktop compositing | Fallback, CI testing |
| virgl (VirtIO 3D) | 30-60 FPS depending on host GPU | Development, QEMU testing |
| Native DRM (i915/amdgpu) | 60+ FPS | Real hardware (future) |

KWin's compositor can run in "basic" mode (no fancy effects) for llvmpipe, or full compositing for virgl/native.

### D-Bus Performance

Standard D-Bus uses Unix domain socket transport with serialized messages. For VeridianOS, a potential optimization is a fast-path backend using the kernel's zero-copy IPC:

```
Standard D-Bus path:
  Client → serialize → send(unix_socket) → dbus-daemon → recv → deserialize → route → send → Client

Optimized VeridianOS path (future):
  Client → serialize → shared_memory_region + notify → dbus-daemon → zero-copy route → Client
```

This optimization is not required for initial porting but could improve Plasma responsiveness significantly.

### Compositor Efficiency

KWin on Wayland is more efficient than X11 compositors:
- Direct scanout: Fullscreen applications bypass compositing entirely
- Damage tracking: Only changed regions are re-rendered
- Explicit sync: GPU synchronization without CPU stalls (when supported)
- Smart repaint: Skip compositing when nothing changed

### Performance Targets

| Metric | Target | Measurement |
|--------|--------|-------------|
| Boot to desktop | < 10 seconds | From KWin start to Plasma shell rendered |
| Input latency | < 16 ms | Key press to screen update (60 FPS frame) |
| Window open | < 500 ms | Click launcher → window fully rendered |
| Compositor FPS | 60 FPS (virgl) | Steady state with 3-5 windows |
| RAM usage | < 1 GB | Full Plasma session with 2 apps |
| D-Bus round-trip | < 1 ms | Method call + reply |

---

## 11. Timeline and Effort

### Sprint Timeline

```
Sprint 9.0  ████████████████ (6-8 weeks)  Dynamic Linking + libc + C++ Runtime
Sprint 9.1  ████████████████ (6-8 weeks)  User-Space Graphics (DRM/KMS, evdev, libinput)
Sprint 9.2  ████████████ (4-6 weeks)      System Libraries
Sprint 9.3  ████████████ (4-6 weeks)      Mesa/EGL
Sprint 9.4  ████████ (3-4 weeks)          Font/Text Stack
Sprint 9.5  ████████████ (4-6 weeks)      D-Bus + Session Management
Sprint 9.6  ████████████████ (6-8 weeks)  Qt 6 Core Port
Sprint 9.7  ████████████████ (6-8 weeks)  KDE Frameworks 6
Sprint 9.8  ████████████ (4-6 weeks)      KWin Compositor
Sprint 9.9  ████████████ (4-6 weeks)      Plasma Desktop
Sprint 9.10 ████████████ (4-6 weeks)      Integration + Polish
            ├────────────────────────────────────────────────┤
            0                    12                    24 months
```

### Critical Path

The critical path determines the minimum possible timeline:

```
Sprint 9.0 (Dynamic Linking) → Sprint 9.3 (Mesa) → Sprint 9.6 (Qt 6) → Sprint 9.8 (KWin) → Sprint 9.10 (Integration)
     8 weeks            +        6 weeks       +       8 weeks      +       6 weeks      +        6 weeks
                                                                                         = 34 weeks minimum
```

With parallelism (Sprints 9.2/9.4/9.5 can overlap with 9.1/9.3):

```
                 Sprint 9.0 (8w)
                      │
            ┌─────────┼─────────┐
            │         │         │
       Sprint 9.1   Sprint 9.2  Sprint 9.4
       (8w)         (6w)        (4w)
            │         │         │
            │    Sprint 9.3     │
            │    (6w)           │
            │         │    Sprint 9.5
            │         │    (6w)
            │         │         │
            └─────────┼─────────┘
                      │
                 Sprint 9.6 (8w)
                      │
                 Sprint 9.7 (8w)
                      │
                 Sprint 9.8 (6w)
                      │
                 Sprint 9.9 (6w)
                      │
                 Sprint 9.10 (6w)
```

**Optimistic**: ~18 months (with parallelism, experienced team)
**Realistic**: ~24 months (single developer, some setback buffer)

### Risk Register

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Mesa port fails (too many Linux assumptions) | HIGH | MEDIUM | Start with llvmpipe only; virgl as stretch goal. Alternative: SwiftShader (Vulkan software) |
| Qt 6 QPA plugin complexity | HIGH | LOW | Qt's QPA is well-documented; WASM/Haiku ports are precedent |
| D-Bus performance insufficient | MEDIUM | LOW | Optimize transport, increase buffer sizes, batch signals |
| libstdc++ exception handling | HIGH | MEDIUM | Start with -fno-exceptions build to unblock Qt, add later |
| KWin DRM backend needs atomic modesetting | MEDIUM | LOW | KWin supports legacy mode-setting fallback |
| Font rendering quality | LOW | LOW | FreeType is battle-tested; fontconfig configuration is the main challenge |
| Kernel heap insufficient | MEDIUM | HIGH | Already known: adjust heap size or implement demand paging |
| Total disk space for KDE rootfs | MEDIUM | MEDIUM | Current BlockFS is 512MB; KDE needs ~2GB. Expand or use ext4 |
| Upstream KDE API changes during port | MEDIUM | MEDIUM | Pin to specific KDE release (e.g., Plasma 6.2 LTS) |

### Effort Summary

| Sprint | Tasks | Effort (weeks) | Category |
|--------|-------|----------------|----------|
| 9.0 | ~20 | 6-8 | Foundation |
| 9.1 | ~18 | 6-8 | Foundation |
| 9.2 | ~15 | 4-6 | Libraries |
| 9.3 | ~12 | 4-6 | Graphics |
| 9.4 | ~12 | 3-4 | Text |
| 9.5 | ~15 | 4-6 | Services |
| 9.6 | ~20 | 6-8 | Qt |
| 9.7 | ~22 | 6-8 | KDE |
| 9.8 | ~15 | 4-6 | Compositor |
| 9.9 | ~15 | 4-6 | Desktop |
| 9.10 | ~15 | 4-6 | Polish |
| **Total** | **~179** | **52-70** | |

---

## Appendix A: Reference Ports

These OS projects have successfully ported KDE/Qt and can serve as references:

| Project | What They Ported | Key Learnings |
|---------|-----------------|---------------|
| **Haiku** | Qt 6 + partial KDE Frameworks | Custom QPA plugin (`haiku`), no Wayland, direct framebuffer |
| **FreeBSD** | Full KDE Plasma 6 | Minimal patches (POSIX compliant), linuxkpi for DRM drivers |
| **Managarm** | Qt 5 + partial KDE | Custom microkernel, mlibc, POSIX shims, helpful for gap analysis |
| **SerenityOS** | Custom DE (Ladybird browser) | Dynamic linker, libc, Mesa (llvmpipe), font stack -- similar scope |

## Appendix B: File Path Quick Reference

| VeridianOS Component | Path |
|---------------------|------|
| Dynamic linker | `userland/ld-veridian/ld-veridian.c` |
| libc sources | `userland/libc/src/*.c` (22 files) |
| dlopen/dlsym stubs | `userland/libc/src/posix_stubs.c:596-616` |
| Kernel Wayland compositor | `kernel/src/desktop/wayland/` (10 files) |
| User-space Wayland client | `userland/libwayland/wayland-client.c` |
| GPU/DRM framework | `kernel/src/graphics/gpu_accel.rs` |
| Unix domain sockets | `kernel/src/net/unix_socket.rs` |
| CMake toolchain generator | `kernel/src/pkg/sdk/toolchain.rs:480-541` |
| Desktop renderer | `kernel/src/desktop/renderer.rs` |
| Init system | `kernel/src/services/init.rs` |
| Package manager | `kernel/src/pkg/` |
| Software porting guide | `docs/SOFTWARE-PORTING-GUIDE.md` |

## Appendix C: Glossary

| Term | Definition |
|------|-----------|
| **DRM** | Direct Rendering Manager -- kernel interface for GPU access |
| **KMS** | Kernel Mode Setting -- display mode configuration via DRM |
| **GBM** | Generic Buffer Management -- buffer allocation API for EGL/DRM |
| **EGL** | Embedded-profile Graphics Library -- GL context/surface management |
| **QPA** | Qt Platform Abstraction -- plugin interface for OS integration |
| **ECM** | Extra CMake Modules -- KDE's CMake macro collection |
| **KF6** | KDE Frameworks 6 -- modular libraries built on Qt 6 |
| **virgl** | Virtual OpenGL -- VirtIO 3D GPU protocol for QEMU |
| **llvmpipe** | LLVM-based software rasterizer in Mesa |
| **SCM_RIGHTS** | Socket Control Message for passing file descriptors over Unix sockets |
| **evdev** | Event device -- Linux/VeridianOS input device interface |

---

**See Also**: [Phase 9 TODO](../to-dos/PHASE9_TODO.md) | [Software Porting Guide](SOFTWARE-PORTING-GUIDE.md) | [Performance Report](PERFORMANCE-REPORT.md)
