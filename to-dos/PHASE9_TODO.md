# Phase 9: KDE Plasma 6 Desktop Environment TODO

**Phase Duration**: 18-24 months
**Status**: In Progress (Sprints 9.0-9.1 complete)
**Dependencies**: Phase 8 (next-generation features)
**Last Updated**: March 7, 2026

## Overview

Phase 9 ports the complete KDE Plasma 6 desktop environment to VeridianOS, from dynamic linking infrastructure through a fully functional Plasma session with KWin compositor, Breeze theming, and core KDE applications. The existing built-in desktop environment is preserved as a lightweight fallback session type. Items are organized into 11 sprints (~179 tasks) with strict dependency ordering. Initial target is x86_64 only.

**Design Reference**: [KDE Plasma Porting Guide](../docs/KDE-PLASMA-PORTING-GUIDE.md)

---

## Sprint 9.0: Dynamic Linking + libc Extensions + C++ Runtime

**Duration**: 6-8 weeks | **Priority**: CRITICAL | **Blocks**: All subsequent sprints

### 9.0.1 Dynamic Linker (ld-veridian)

- [x] Test ld-veridian with trivial shared object (compile libtest.so, load, call symbol)
- [x] Wire dlopen() to ld-veridian ELF loader (replace stub in `posix_stubs.c:596`)
- [x] Wire dlsym() to ld-veridian symbol lookup (replace stub in `posix_stubs.c:603`)
- [x] Wire dlclose() to ld-veridian unload (replace stub in `posix_stubs.c:610`)
- [x] Implement dlerror() with thread-local error string
- [x] Implement dladdr() (symbol address lookup for backtraces)
- [x] Implement dl_iterate_phdr() (shared object enumeration for exception handling)
- [x] Add RTLD_GLOBAL / RTLD_LOCAL / RTLD_NOW / RTLD_LAZY flag support
- [x] Implement lazy binding (PLT stubs with first-call resolution)
- [x] Implement GNU hash (DT_GNU_HASH) for fast symbol lookup
- [x] Implement symbol versioning (DT_VERSYM, DT_VERDEF, DT_VERNEED)
- [x] Implement DT_NEEDED recursive loading (transitive dependency resolution)
- [x] Implement init/fini arrays (DT_INIT_ARRAY, DT_FINI_ARRAY execution order)
- [x] Implement DT_RPATH / DT_RUNPATH / LD_LIBRARY_PATH search order
- [x] Test with chain of 3+ shared objects with transitive dependencies
- [x] Install as `/lib/ld-veridian.so.1` on rootfs

### 9.0.2 libc Extensions

- [x] Implement iconv (character set conversion -- UTF-8/Latin-1/UCS-2 minimum)
- [x] Implement locale support (setlocale, localeconv, LC_* categories)
- [x] Implement getpwuid / getpwnam / getgrgid / getgrnam (user/group database)
- [x] Implement mmap MAP_SHARED (shared memory mappings between processes)
- [x] Implement epoll (epoll_create, epoll_ctl, epoll_wait -- event loop)
- [x] Implement atexit / __cxa_atexit (C++ static destructor registration)
- [x] Implement posix_memalign / aligned_alloc (Mesa requires aligned allocation)
- [x] Implement clock_gettime with CLOCK_MONOTONIC and CLOCK_REALTIME
- [x] Implement getaddrinfo / freeaddrinfo (DNS resolution for Qt Network)
- [x] Implement sysconf (_SC_NPROCESSORS_ONLN, _SC_PAGE_SIZE, _SC_OPEN_MAX)
- [x] Verify poll() and select() work with pipes, sockets, and device fds

### 9.0.3 C++ Runtime (libstdc++)

- [x] Port libgcc_s (GCC runtime: __divdi3, __moddi3, stack unwinding tables)
- [x] Port libunwind (DWARF .eh_frame unwinder for C++ exceptions)
- [x] Build libstdc++ with --enable-shared for VeridianOS target
- [x] Verify C++ exception throw/catch (try/catch, std::exception hierarchy)
- [x] Verify RTTI (dynamic_cast, typeid)
- [x] Verify std::thread (backed by pthread_create)
- [x] Verify std::mutex, std::condition_variable (backed by pthread_mutex/cond)
- [x] Verify std::filesystem (backed by POSIX open/read/stat/readdir)
- [x] Verify std::chrono (backed by clock_gettime)
- [x] Verify STL containers (vector, map, unordered_map, string) under dynamic linking
- [x] Install libstdc++.so.6 and libgcc_s.so.1 in sysroot

---

## Sprint 9.1: User-Space Graphics (DRM/KMS, evdev, libinput)

**Duration**: 6-8 weeks | **Priority**: CRITICAL | **Blocks**: Sprint 9.3 (Mesa), Sprint 9.8 (KWin)

### 9.1.1 DRM/KMS Device Interface

- [x] Create `/dev/dri/card0` device node backed by VirtIO GPU driver
- [x] Create `/dev/dri/renderD128` render node for Mesa DRI
- [x] Implement DRM_IOCTL_VERSION (driver name, version, description)
- [x] Implement DRM_IOCTL_GET_CAP (query driver capabilities)
- [x] Implement DRM_IOCTL_MODE_GETRESOURCES (enumerate CRTCs, connectors, encoders)
- [x] Implement DRM_IOCTL_MODE_GETCONNECTOR (connector state, modes, properties)
- [x] Implement DRM_IOCTL_MODE_GETENCODER (encoder-to-CRTC mapping)
- [x] Implement DRM_IOCTL_MODE_GETCRTC / SETCRTC (display mode configuration)
- [x] Implement DRM_IOCTL_MODE_PAGE_FLIP (vsync-aware buffer swap)
- [x] Implement DRM_IOCTL_MODE_CREATE_DUMB / MAP_DUMB / DESTROY_DUMB (dumb buffers)
- [x] Implement DRM_IOCTL_GEM_CLOSE (GEM handle lifecycle)
- [x] Implement DRM_IOCTL_PRIME_HANDLE_TO_FD / FD_TO_HANDLE (DMA-BUF sharing)
- [x] Implement DRM_IOCTL_SET_MASTER / DROP_MASTER (compositor exclusivity)
- [x] Implement mmap for DRM dumb buffer mapping to user-space
- [x] Implement vblank event delivery via DRM fd (poll/epoll readable on vblank)
- [x] Test with libdrm's modetest utility

### 9.1.2 VirtIO GPU DRM Driver

- [x] Bridge kernel VirtIO GPU 3D protocol to DRM ioctl interface
- [x] Map VirtIO GPU resources to GEM handles
- [x] Implement VirtIO GPU SUBMIT_3D for virgl command streams
- [x] Implement VirtIO GPU TRANSFER_TO/FROM_HOST for buffer synchronization
- [x] Handle VirtIO GPU fence completion as DRM sync events

### 9.1.3 evdev Input Interface

- [x] Create `/dev/input/event*` device nodes for keyboard and mouse
- [x] Implement `struct input_event` (type, code, value, time) read interface
- [x] Implement EVIOCGNAME (device name query)
- [x] Implement EVIOCGBIT (capability bits -- EV_KEY, EV_REL, EV_ABS)
- [x] Implement EVIOCGABS (absolute axis info for touchpads/tablets)
- [x] Implement EVIOCGRAB (exclusive input device grab)
- [x] Route VirtIO input events to evdev device nodes
- [x] Route PS/2 keyboard events to evdev device nodes
- [x] Implement multi-device support (separate event nodes per device)

### 9.1.4 libdrm Port

- [x] Cross-compile libdrm with Meson for VeridianOS
- [x] Verify drmModeGetResources / drmModeGetConnector
- [x] Verify drmModeSetCrtc / drmModePageFlip
- [x] Verify drmPrimeHandleToFD / drmPrimeFDToHandle
- [x] Verify GBM (gbm_create_device, gbm_surface_create, gbm_bo_*)
- [x] Install libdrm.so and libgbm.so in sysroot

### 9.1.5 libinput Port

- [x] Implement udev shim or minimal device discovery for libinput
- [x] Cross-compile libinput with Meson for VeridianOS
- [x] Verify pointer motion and button events
- [x] Verify keyboard key events
- [x] Verify pointer acceleration profiles
- [x] Verify scroll events (button, edge, two-finger)
- [x] Install libinput.so in sysroot

---

## Sprint 9.2: System Libraries

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6)

### 9.2.1 Core Libraries

- [ ] Cross-compile zlib 1.3.x (CMake, `--host=x86_64-veridian`)
- [ ] Cross-compile expat 2.6.x (CMake)
- [ ] Cross-compile libffi 3.4.x (Autotools, x86_64 assembly backend)
- [ ] Cross-compile double-conversion 3.3.x (CMake)
- [ ] Cross-compile pcre2 10.43 (CMake, JIT disabled initially)
- [ ] Cross-compile ICU 75.x (Autotools, requires host ICU for data generation)
- [ ] Cross-compile OpenSSL 3.3.x (custom build, `Configure VeridianOS-x86_64`, no-asm initially)
- [ ] Cross-compile libxml2 2.13.x (CMake, with ICU + iconv)

### 9.2.2 Image Libraries

- [ ] Cross-compile libjpeg-turbo 3.0.x (CMake, SIMD disabled initially)
- [ ] Cross-compile libpng 1.6.x (CMake, depends on zlib)
- [ ] Cross-compile libtiff 4.6.x (CMake, depends on zlib + libjpeg-turbo)
- [ ] Cross-compile libwebp 1.4.x (CMake, optional -- for KDE image support)

### 9.2.3 Validation

- [ ] Verify all libraries install to sysroot with .pc files
- [ ] Run basic test programs linking against each library in QEMU
- [ ] Verify pkg-config cross-compilation wrapper finds all libraries

---

## Sprint 9.3: Mesa / EGL / OpenGL

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 rendering), Sprint 9.8 (KWin)

### 9.3.1 Mesa Port

- [ ] Create Meson cross-file for VeridianOS (veridian-cross.ini)
- [ ] Configure Mesa with `-Dgallium-drivers=swrast,virgl` (llvmpipe + virgl)
- [ ] Configure Mesa with `-Dplatforms=wayland` (EGL wayland platform)
- [ ] Configure Mesa with `-Degl=enabled -Dgles2=enabled -Dglx=disabled`
- [ ] Implement VeridianOS DRM loader shim (`src/loader/loader.c` patches)
- [ ] Implement VeridianOS platform shim for `egl_dri2.c`
- [ ] Patch thread primitives (pthread_*, TLS) for VeridianOS
- [ ] Patch mmap usage for VeridianOS (MAP_SHARED, MAP_ANONYMOUS)
- [ ] Build Mesa (expect 20-30 min compile time)
- [ ] Verify EGL initialization with llvmpipe (software rendering)
- [ ] Verify EGL initialization with virgl (VirtIO GPU 3D)
- [ ] Verify eglCreateWindowSurface with Wayland display

### 9.3.2 libepoxy Port

- [ ] Cross-compile libepoxy 1.5.x (Meson)
- [ ] Verify GL/EGL function dispatch
- [ ] Install libepoxy.so in sysroot

### 9.3.3 Validation

- [ ] Run eglinfo on VeridianOS (verify EGL vendor, version, extensions)
- [ ] Run es2_info (verify GLES2 renderer string, extensions)
- [ ] Render a spinning triangle with EGL + GLES2 in QEMU
- [ ] Verify virgl acceleration works (`LIBGL_ALWAYS_SOFTWARE=0`)

---

## Sprint 9.4: Font / Text Stack

**Duration**: 3-4 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 GUI)

### 9.4.1 Font Libraries

- [ ] Cross-compile FreeType 2.13.x (CMake, with zlib + libpng)
- [ ] Cross-compile HarfBuzz 9.0.x (Meson, with FreeType + ICU)
- [ ] Cross-compile Fontconfig 2.15.x (Meson, with FreeType + expat)
- [ ] Create `/etc/fonts/fonts.conf` with VeridianOS font paths
- [ ] Create `/etc/fonts/conf.d/` with default font matching rules

### 9.4.2 Font Packages

- [ ] Install DejaVu Sans / Sans Mono / Serif fonts to `/usr/share/fonts/dejavu/`
- [ ] Install Noto Sans / Serif / Mono fonts to `/usr/share/fonts/noto/`
- [ ] Install Liberation Sans / Serif / Mono fonts to `/usr/share/fonts/liberation/`
- [ ] Install Noto Sans CJK fonts (optional, ~100MB)
- [ ] Generate fontconfig cache (`fc-cache -f` or pre-built cache)

### 9.4.3 libxkbcommon Port

- [ ] Cross-compile libxkbcommon 1.7.x (Meson, with wayland support)
- [ ] Install XKB data files (`/usr/share/X11/xkb/`)
- [ ] Verify keymap compilation (us, de, fr layouts)
- [ ] Verify compose key support
- [ ] Install libxkbcommon.so in sysroot

### 9.4.4 Validation

- [ ] Render "Hello VeridianOS" with FreeType in a test program
- [ ] Verify fontconfig finds installed fonts (`fc-list` equivalent)
- [ ] Verify HarfBuzz shapes Latin + CJK text correctly

---

## Sprint 9.5: D-Bus + Session Management

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 DBus), Sprint 9.7 (KDE)

### 9.5.1 D-Bus Reference Implementation

- [ ] Cross-compile dbus-1 1.15.x (Meson) for VeridianOS
- [ ] Patch Unix socket transport for VeridianOS (should mostly work)
- [ ] Patch user/group lookup to use VeridianOS getpwuid/getgrgid
- [ ] Configure system bus (`/var/run/dbus/system_bus_socket`)
- [ ] Configure session bus (`$XDG_RUNTIME_DIR/bus`)
- [ ] Implement D-Bus activation (launching services on demand via .service files)
- [ ] Implement signal matching and property interface
- [ ] Implement introspection interface (org.freedesktop.DBus.Introspectable)
- [ ] Create init system integration (start dbus-daemon at boot)
- [ ] Verify dbus-send / dbus-monitor work in QEMU

### 9.5.2 logind Shim

- [ ] Implement org.freedesktop.login1.Manager (GetSession, GetSeat)
- [ ] Implement org.freedesktop.login1.Session (TakeDevice, ReleaseDevice)
- [ ] Implement TakeControl / ReleaseControl (compositor exclusivity)
- [ ] Implement session Active property and Activate method
- [ ] Implement PauseDevice / ResumeDevice signals (VT switching)
- [ ] Map TakeDevice to VeridianOS capability-based device access
- [ ] Implement SetIdleHint for idle detection
- [ ] Register as D-Bus system service
- [ ] Test with KWin's logind session backend

### 9.5.3 Polkit

- [ ] Cross-compile Polkit 124.x (Meson, with D-Bus)
- [ ] Configure default authorization rules
- [ ] Register as D-Bus system service (org.freedesktop.PolicyKit1)
- [ ] Verify pkexec basic operation

### 9.5.4 Validation

- [ ] D-Bus system bus starts at boot and accepts connections
- [ ] D-Bus session bus starts on user login
- [ ] logind shim provides session and device access
- [ ] dbus-send can call logind methods

---

## Sprint 9.6: Qt 6 Core Port

**Duration**: 6-8 weeks | **Priority**: CRITICAL | **Blocks**: Sprint 9.7 (KDE Frameworks)

### 9.6.1 Qt 6 Build System

- [ ] Build Qt 6 host tools (moc, rcc, uic, qsb, qlalr) on Linux host
- [ ] Create CMake cross-compilation toolchain for Qt 6 (qt.toolchain.cmake)
- [ ] Create `cmake/platforms/VeridianOS.cmake` platform configuration
- [ ] Create mkspec `mkspecs/veridian-g++/qmake.conf`
- [ ] Configure Qt 6 with `-platform veridian-g++ -xplatform veridian-g++`

### 9.6.2 QPA Plugin (qveridian)

- [ ] Implement QPlatformIntegration (screen, clipboard, services, theme)
- [ ] Implement QPlatformScreen (geometry, depth, format, refresh rate from DRM)
- [ ] Implement QPlatformWindow (Wayland surface create/destroy/resize)
- [ ] Implement QPlatformBackingStore (SHM or EGL buffer management)
- [ ] Implement QPlatformOpenGLContext (EGL context creation, makeCurrent, swapBuffers)
- [ ] Implement QPlatformInputContext (keyboard/mouse event dispatch)
- [ ] Implement QPlatformClipboard (Wayland clipboard protocol)
- [ ] Implement QPlatformTheme (font database, icons, standard dialogs)
- [ ] Register plugin as `platforms/libqveridian.so`

### 9.6.3 QtCore

- [ ] Port QEventDispatcherUNIX (epoll-based event loop)
- [ ] Port QThread / QMutex / QWaitCondition (pthread backend)
- [ ] Port QProcess (fork/exec, pipe, waitpid)
- [ ] Port QFileSystemEngine (POSIX stat, readdir, access)
- [ ] Port QLocale (locale database, number/date formatting)
- [ ] Port QTimeZone (timezone database, /usr/share/zoneinfo or embedded)
- [ ] Port QSocketNotifier (epoll fd monitoring)
- [ ] Port QSharedMemory / QSystemSemaphore (POSIX shm_open, sem_open)
- [ ] Verify QTimer, QCoreApplication event loop
- [ ] Verify QSettings (INI/registry backend)

### 9.6.4 QtGui

- [ ] Port QFontDatabase (FreeType + Fontconfig backend)
- [ ] Port QPlatformFontDatabase (font enumeration, family matching)
- [ ] Port QImage / QPixmap (image loading via libjpeg/libpng)
- [ ] Verify text rendering with FreeType + HarfBuzz shaping
- [ ] Verify EGL/OpenGL rendering via QPA plugin
- [ ] Verify Wayland window creation and buffer attachment

### 9.6.5 QtWidgets

- [ ] Build QtWidgets module
- [ ] Verify QPushButton, QLabel, QLineEdit, QTextEdit rendering
- [ ] Verify QMainWindow, QMenuBar, QToolBar, QStatusBar
- [ ] Verify QDialog (modal, file dialog, message box)
- [ ] Verify QTreeView, QListView with models

### 9.6.6 QtQml / QtQuick

- [ ] Build QtShaderTools (SPIR-V cross-compiler for scene graph)
- [ ] Build QtDeclarative (Qml, Quick) with JIT disabled (interpreter mode)
- [ ] Verify QML file loading and component instantiation
- [ ] Verify Qt Quick scene graph with OpenGL ES 2.0
- [ ] Verify basic animations (PropertyAnimation, NumberAnimation)

### 9.6.7 QtWayland

- [ ] Build QtWayland client module (links against libwayland-client)
- [ ] Verify xdg-shell integration (toplevel, popup)
- [ ] Verify xdg-decoration protocol (server-side decorations)
- [ ] Verify surface damage and buffer commit
- [ ] Verify keyboard/pointer input via Wayland seat

### 9.6.8 Additional Qt Modules

- [ ] Build QtDBus (links against libdbus-1, session bus connection)
- [ ] Build QtNetwork (SSL via OpenSSL, DNS via getaddrinfo)
- [ ] Build QtSvg (SVG rendering for icons)
- [ ] Build Qt5Compat (text codecs for legacy KDE code)

### 9.6.9 Validation

- [ ] Run Qt 6 auto-tests subset (core, gui, widgets) in QEMU
- [ ] Launch a simple Qt 6 Widgets application (window, button, text)
- [ ] Launch a simple Qt Quick application (QML loader, animation)
- [ ] Verify D-Bus connection from QtDBus
- [ ] Screenshot test: Qt 6 test app renders correctly

---

## Sprint 9.7: KDE Frameworks 6

**Duration**: 6-8 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.8 (KWin), Sprint 9.9 (Plasma)

### 9.7.1 Extra CMake Modules (ECM)

- [ ] Port ECM 6.x with VeridianOS platform detection
- [ ] Add VeridianOS to KDEInstallDirs (install prefix, lib dir, plugin dir)
- [ ] Verify KDECompilerSettings applies correct flags for VeridianOS
- [ ] Install ECM in sysroot cmake directory

### 9.7.2 Tier 1 Frameworks (No KDE Dependencies)

- [ ] Build KConfig (configuration management)
- [ ] Build KCoreAddons (core utilities, KAboutData, KPluginLoader)
- [ ] Build KI18n (internationalization, gettext wrapper)
- [ ] Build KWidgetsAddons (additional Qt widgets)
- [ ] Build KDBusAddons (D-Bus utilities)
- [ ] Build KGuiAddons (color, font, key sequence helpers)
- [ ] Build KItemViews (enhanced item views)
- [ ] Build KItemModels (proxy models)
- [ ] Build KColorScheme (color scheme management)
- [ ] Build Solid (hardware device discovery -- needs device shim)
  - [ ] Implement VeridianOS backend for Solid (device enumeration, properties)
  - [ ] Map VeridianOS device capabilities to Solid device types
- [ ] Build Sonnet (spell checking -- hunspell backend)
- [ ] Build KArchive (tar, zip, gzip)
- [ ] Build KCodecs (base64, quoted-printable, UUencode)
- [ ] Build KCompletion (text completion)
- [ ] Build ThreadWeaver (multi-threaded jobs)
- [ ] Verify all Tier 1 auto-tests pass

### 9.7.3 Tier 2 Frameworks (Depend on Tier 1)

- [ ] Build KNotifications (desktop notifications via D-Bus)
- [ ] Build KXmlGui (XML-based menu/toolbar construction)
- [ ] Build KIconThemes (icon theme engine, SVG rendering)
- [ ] Build KConfigWidgets (widgets for KConfig)
- [ ] Build KGlobalAccel (global keyboard shortcuts via D-Bus)
- [ ] Build KCrash (crash handling framework)
- [ ] Build KAuth (authorization via Polkit)
- [ ] Build KJobWidgets (job progress widgets)
- [ ] Build KBookmarks (bookmark management)
- [ ] Verify all Tier 2 auto-tests pass

### 9.7.4 Tier 3 Frameworks (Depend on Tier 1+2)

- [ ] Build KIO (virtual filesystem, network I/O)
  - [ ] Implement VeridianOS KIO worker for local filesystem
  - [ ] Verify file://, trash:// protocols
- [ ] Build KWindowSystem (window management, Wayland backend)
  - [ ] Implement Wayland backend using KDE Wayland protocols
- [ ] Build KNewStuff (content download -- optional initially)
- [ ] Build KService (service/plugin discovery, .desktop file parsing)
- [ ] Build KParts (document component framework)
- [ ] Build KTextWidgets (rich text editing widgets)
- [ ] Build KWallet (credential storage -- simple file backend initially)
- [ ] Build KDeclarative (QML integration for KDE)
- [ ] Build Plasma Framework (containments, applets, DataEngine)
- [ ] Build KPackage (package format for Plasma add-ons)
- [ ] Build KActivities (activity/virtual desktop management)
- [ ] Verify all Tier 3 auto-tests pass

---

## Sprint 9.8: KWin Compositor

**Duration**: 4-6 weeks | **Priority**: CRITICAL | **Blocks**: Sprint 9.9 (Plasma Desktop)

### 9.8.1 KWin Core

- [ ] Build KWin with CMake for VeridianOS
- [ ] Configure DRM/KMS backend (libdrm + GBM)
- [ ] Configure libinput input backend
- [ ] Configure logind session backend (via D-Bus)
- [ ] Configure EGL/OpenGL ES 2.0 compositing
- [ ] Implement VeridianOS-specific platform adjustments (if any)

### 9.8.2 KWin Wayland Server

- [ ] Verify KWin creates Wayland socket (`$XDG_RUNTIME_DIR/wayland-0`)
- [ ] Verify xdg-shell protocol (toplevel window management)
- [ ] Verify xdg-decoration protocol (server-side window decorations)
- [ ] Verify wlr-layer-shell protocol (panels, overlays)
- [ ] Verify Wayland seat (keyboard + pointer events to clients)
- [ ] Verify surface damage and compositing
- [ ] Verify multi-window management (focus, stacking, move, resize)

### 9.8.3 KDE Wayland Protocol Extensions

- [ ] Verify org_kde_plasma_shell protocol (desktop/panel surface roles)
- [ ] Verify org_kde_plasma_window_management (window list for taskbar)
- [ ] Verify org_kde_kwin_server_decoration_manager (decoration negotiation)
- [ ] Verify org_kde_kwin_blur_manager (background blur effect)
- [ ] Verify org_kde_kwin_dpms (display power management)
- [ ] Verify org_kde_kwin_outputdevice + outputmanagement (display config)

### 9.8.4 KWin Effects

- [ ] Verify basic effects (minimize animation, window snap, desktop grid)
- [ ] Verify Blur effect with OpenGL ES
- [ ] Verify Overview effect (workspace overview)
- [ ] Disable unsupported effects gracefully (GPU-intensive ones on llvmpipe)

### 9.8.5 Validation

- [ ] KWin starts standalone (without Plasma shell)
- [ ] KWin renders a solid background and accepts Wayland clients
- [ ] Launch a Qt 6 test application under KWin (window decorations visible)
- [ ] Verify keyboard and mouse input through KWin
- [ ] Measure frame rate (target: 60 FPS with virgl, 15+ FPS with llvmpipe)

---

## Sprint 9.9: Plasma Desktop

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.10 (Integration)

### 9.9.1 Plasma Workspace

- [ ] Build plasma-workspace (startkde, session management, lock screen)
- [ ] Build plasma-desktop (desktop containment, folder view)
- [ ] Build plasma-integration (Qt platform theme for Plasma)
- [ ] Implement VeridianOS session startup script (init → D-Bus → KWin → Plasma)
- [ ] Configure XDG_RUNTIME_DIR, XDG_CONFIG_HOME, XDG_DATA_HOME

### 9.9.2 Breeze Theme

- [ ] Build kdecoration (window decoration framework)
- [ ] Build Breeze Qt style (widget theming)
- [ ] Build Breeze window decoration (title bar, buttons)
- [ ] Install Breeze icon theme to `/usr/share/icons/breeze/`
- [ ] Install Breeze cursor theme to `/usr/share/icons/breeze_cursors/`
- [ ] Install Breeze color scheme as default
- [ ] Install Plasma wallpaper to `/usr/share/wallpapers/`

### 9.9.3 Plasma Shell Components

- [ ] Verify panel (taskbar) renders and shows running windows
- [ ] Verify application launcher (Kickoff) opens and lists apps
- [ ] Verify system tray (clock, volume, network, notifications)
- [ ] Verify desktop right-click context menu
- [ ] Verify virtual desktop switching
- [ ] Verify notification system (D-Bus notifications displayed as popups)

### 9.9.4 System Settings

- [ ] Build KScreen (display configuration KCM)
- [ ] Build PowerDevil (power management KCM)
- [ ] Build System Settings application (KDE systemsettings6)
- [ ] Verify at least 5 KCMs load and display correctly

### 9.9.5 Core KDE Applications

- [ ] Build Dolphin (file manager -- depends on KIO, KParts)
- [ ] Build Konsole (terminal emulator -- depends on KParts, PTY)
- [ ] Build Kate (text editor -- depends on KTextEditor, KParts)
- [ ] Build Spectacle (screenshot utility -- depends on KWin D-Bus)
- [ ] Verify each application launches, renders, and responds to input
- [ ] Create .desktop files for application launcher integration

### 9.9.6 Validation

- [ ] Full Plasma session boots to desktop with panel and wallpaper
- [ ] Application launcher shows installed apps
- [ ] Dolphin browses filesystem
- [ ] Konsole opens shell with working input/output
- [ ] Kate opens and edits a text file
- [ ] Screenshot test: Plasma desktop matches reference image

---

## Sprint 9.10: Integration + Polish

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Release

### 9.10.1 Boot Sequence

- [ ] Update init system to start D-Bus system bus at boot
- [ ] Update init system to start logind shim at boot
- [ ] Create session startup script: D-Bus session → KWin → Plasma shell
- [ ] Implement session type selection (built-in DE vs. KDE Plasma) at login
- [ ] Verify clean shutdown sequence (Plasma → KWin → D-Bus → halt)
- [ ] Measure boot-to-desktop time (target: < 10 seconds from KWin start)

### 9.10.2 Session Switching

- [ ] Implement display manager or login prompt with session type menu
- [ ] Built-in DE: starts kernel compositor (existing code path)
- [ ] KDE Plasma: starts KWin + Plasma in user-space (new code path)
- [ ] Verify switching between session types across reboots
- [ ] Verify built-in DE still works (regression test)

### 9.10.3 XWayland

- [ ] Port XWayland (Xwayland server binary) for X11 app compatibility
- [ ] Configure KWin to launch Xwayland on demand
- [ ] Verify X11 application renders under KWin (e.g., xterm, xclock)
- [ ] Verify clipboard sharing between Wayland and X11 apps
- [ ] Verify input forwarding to X11 apps

### 9.10.4 Performance Optimization

- [ ] Profile Plasma session memory usage (target: < 1 GB total)
- [ ] Profile compositor frame timing (target: 60 FPS with virgl)
- [ ] Profile input latency (target: < 16 ms key-to-screen)
- [ ] Profile D-Bus round-trip latency (target: < 1 ms)
- [ ] Optimize font cache warming (pre-generate fontconfig cache)
- [ ] Profile and optimize KWin startup time

### 9.10.5 Disk Image

- [ ] Expand rootfs from 512MB to 2GB+ (KDE requires more space)
- [ ] Package all KDE libraries and applications into rootfs
- [ ] Create QEMU launch script for KDE session
- [ ] Document QEMU flags for KDE testing (VirtIO GPU, 2GB RAM, SMP)

### 9.10.6 Testing and Regression

- [ ] Verify all 4,095 existing kernel tests still pass
- [ ] Run KDE Frameworks auto-test suite (subset)
- [ ] Automated screenshot comparison for 5 reference screens
- [ ] Test window management operations (move, resize, minimize, maximize, close)
- [ ] Test multi-window workflow (3+ apps open simultaneously)
- [ ] Test keyboard shortcuts (Alt+Tab, Meta for launcher)
- [ ] Document known limitations and workarounds

### 9.10.7 CI Pipeline Update

- [ ] Add KDE build job to GitHub Actions workflow
- [ ] Cache sysroot layers for incremental builds
- [ ] Add QEMU boot test for KDE session
- [ ] Add screenshot comparison CI step
- [ ] Update CI matrix to include KDE-enabled builds

---

## Progress Tracking

| Sprint | Items | Completed | Status |
|--------|-------|-----------|--------|
| 9.0: Dynamic Linking + libc + C++ | 37 | 0 | Planned |
| 9.1: User-Space Graphics | 35 | 0 | Planned |
| 9.2: System Libraries | 15 | 0 | Planned |
| 9.3: Mesa / EGL | 16 | 0 | Planned |
| 9.4: Font / Text Stack | 15 | 0 | Planned |
| 9.5: D-Bus + Session Mgmt | 17 | 0 | Planned |
| 9.6: Qt 6 Core Port | 35 | 0 | Planned |
| 9.7: KDE Frameworks 6 | 35 | 0 | Planned |
| 9.8: KWin Compositor | 20 | 0 | Planned |
| 9.9: Plasma Desktop | 22 | 0 | Planned |
| 9.10: Integration + Polish | 25 | 0 | Planned |
| **Total** | **~272** | **0** | **0%** |

---

## Dependencies Between Sprints

```
Sprint 9.0 (Foundation) ──→ Sprint 9.1 (Graphics)
         │                         │
         ├──→ Sprint 9.2 (Syslibs) ├──→ Sprint 9.3 (Mesa)
         │         │               │         │
         │         ├──→ Sprint 9.4 (Fonts)   │
         │         │                         │
         ├──→ Sprint 9.5 (D-Bus)             │
         │         │                         │
         │         └─────────┬───────────────┘
         │                   │
         └──→ Sprint 9.6 (Qt 6) ──→ Sprint 9.7 (KDE Frameworks)
                                            │
                                   Sprint 9.8 (KWin)
                                            │
                                   Sprint 9.9 (Plasma Desktop)
                                            │
                                   Sprint 9.10 (Integration)
```

### Parallelizable Sprints

These sprints can be worked on concurrently:
- **Sprint 9.1** (Graphics) and **Sprint 9.2** (System Libraries) -- independent after Sprint 9.0
- **Sprint 9.4** (Fonts) and **Sprint 9.5** (D-Bus) -- independent, both depend on Sprint 9.2
- **Sprint 9.3** (Mesa) and **Sprint 9.4** (Fonts) and **Sprint 9.5** (D-Bus) -- partially overlapping

---

**Previous Phase**: [Phase 8 - Next-Generation Features](PHASE8_TODO.md)
**Design Reference**: [KDE Plasma Porting Guide](../docs/KDE-PLASMA-PORTING-GUIDE.md)
**See Also**: [Master TODO](MASTER_TODO.md) | [Software Porting Guide](../docs/SOFTWARE-PORTING-GUIDE.md)
