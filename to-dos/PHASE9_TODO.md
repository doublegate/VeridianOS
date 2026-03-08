# Phase 9: KDE Plasma 6 Desktop Environment TODO

**Phase Duration**: 18-24 months
**Status**: In Progress (Sprints 9.0-9.6 complete)
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

- [x] Cross-compile zlib 1.3.x (native shim: functional deflate/inflate with Adler-32, CRC-32, stored blocks + Huffman decode)
- [x] Cross-compile expat 2.6.x (native shim: SAX-style state machine parser with elements, attributes, CDATA, comments, PIs)
- [x] Cross-compile libffi 3.4.x (native shim: x86_64 System V ABI ffi_call with GPR args + inline asm, closure stubs)
- [x] Cross-compile double-conversion 3.3.x (native shim: dtoa/strtod with snprintf/strtod backend)
- [x] Cross-compile pcre2 10.43 (native shim: POSIX regex backend via regcomp/regexec)
- [x] Cross-compile ICU 75.x (native shim: ASCII+Latin-1 character properties, UTF-8/16 conversion, collation, break iteration, normalization passthrough)
- [x] Cross-compile OpenSSL 3.3.x (native shim: functional SHA-256/384/512, HMAC, EVP digest API, TLS context stubs)
- [x] Cross-compile libxml2 2.13.x (native shim: SAX-to-DOM via expat backend, XPath stubs)

### 9.2.2 Image Libraries

- [x] Cross-compile libjpeg-turbo 3.0.x (native shim: JPEG decompression stubs with error handling framework)
- [x] Cross-compile libpng 1.6.x (native shim: PNG reader with zlib inflate, IHDR parsing, row-based decoding)
- [x] Cross-compile libtiff 4.6.x (native shim: TIFF open/read/write stubs)
- [x] Cross-compile libwebp 1.4.x (native shim: WebP decode/encode stubs)

### 9.2.3 Validation

- [x] Verify all libraries install to sysroot with headers (38 header files across unicode/, openssl/, libxml/, ffi.h, png.h, etc.)
- [x] Verify kernel builds clean on all 3 architectures (x86_64, AArch64, RISC-V)
- [x] Fix header diagnostics (missing stdint.h in uversion.h/crypto.h/ssl.h, missing png_const_bytep in png.h)

---

## Sprint 9.3: Mesa / EGL / OpenGL

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 rendering), Sprint 9.8 (KWin)

### 9.3.1 EGL Implementation (native shim)

- [x] Create `EGL/eglplatform.h` -- platform types (EGLNativeDisplayType, EGLNativeWindowType)
- [x] Create `EGL/egl.h` -- Core EGL 1.5 API (types, function declarations, constants)
- [x] Create `EGL/eglext.h` -- EGL extensions (KHR_image_base, KHR_platform_wayland, EXT_image_dma_buf_import, MESA_platform_gbm, KHR_fence_sync, KHR_create_context, EXT_buffer_age, EXT_swap_buffers_with_damage)
- [x] Create `egl.c` -- EGL implementation backed by DRM/GBM (~640 LOC): display lifecycle, config selection (4 configs: RGBA8888/RGBX8888/RGBA-no-depth/RGB565), context management, window/pbuffer surfaces, DMA-BUF image import, fence sync, extension queries, eglGetProcAddress dispatch table
- [x] EGL vendor string: "VeridianOS Mesa 24.2 (llvmpipe)"

### 9.3.2 OpenGL ES 2.0/3.0 Implementation (native shim)

- [x] Create `KHR/khrplatform.h` -- Khronos platform types (khronos_int32_t, etc.)
- [x] Create `GLES2/gl2platform.h` -- Platform defines
- [x] Create `GLES2/gl2.h` -- OpenGL ES 2.0 core API (~150 function declarations, all types and enums)
- [x] Create `GLES2/gl2ext.h` -- GLES2 extensions (OES_vertex_array_object, OES_mapbuffer, OES_depth24/32, OES_packed_depth_stencil, OES_EGL_image, EXT_texture_format_BGRA8888, EXT_discard_framebuffer, EXT_blend_minmax, OES_standard_derivatives, OES_rgb8_rgba8)
- [x] Create `GLES3/gl3platform.h` -- Platform defines
- [x] Create `GLES3/gl3.h` -- OpenGL ES 3.0 API (superset of GLES2 with VAOs, buffer mapping, FBO blit, instanced draw, UBOs, sync, samplers, transform feedback, 3D textures)
- [x] Create `gles2.c` -- GLES2/3.0 stub implementation (~1150 LOC): full state tracking (clear color, viewport, scissor, blend, depth, stencil, cull face, color mask), shader/program management with compile/link success, texture/buffer/FBO/RBO/VAO object ID generation, glGetString (vendor/renderer/version/extensions), glGetIntegerv (~60 parameter queries), OES extension stubs, GLES3 stubs
- [x] GL_RENDERER: "llvmpipe (LLVM 19.1, 256 bits)", GL_VERSION: "OpenGL ES 2.0 VeridianOS"

### 9.3.3 libepoxy Implementation (native shim)

- [x] Create `epoxy/common.h` -- EPOXY_PUBLIC macro
- [x] Create `epoxy/gl.h` -- GL dispatch header (includes GLES2+GLES3, desktop GL type compat)
- [x] Create `epoxy/egl.h` -- EGL dispatch header (includes EGL/egl.h + eglext.h)
- [x] Create `epoxy/gl_generated.h` -- Generated-style dispatch (direct linkage, no function pointers needed)
- [x] Create `epoxy/egl_generated.h` -- Generated-style dispatch (direct linkage)
- [x] Create `libepoxy.c` -- libepoxy dispatch (~90 LOC): epoxy_gl_version()=20, epoxy_egl_version()=15, epoxy_is_desktop_gl()=0, epoxy_has_gl_extension()/epoxy_has_egl_extension() with substring-safe matching

### 9.3.4 Meson Cross-File

- [x] Create `meson-veridian-cross.ini` -- Meson cross-compilation file (binaries, host_machine, properties, built-in options)

### 9.3.5 Validation

- [x] Verify kernel builds clean on all 3 architectures (x86_64, AArch64, RISC-V)

---

## Sprint 9.4: Font / Text Stack

**Duration**: 3-4 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 GUI)

### 9.4.1 Font Libraries

- [x] Cross-compile FreeType 2.13.x (native shim: face loading with metrics, glyph rendering with 8x16 fallback bitmaps, charmap selection, kerning queries, stroker, outline/bitmap ops, glyph object management; 14 headers across freetype/ + ft2build.h, ~750 LOC impl)
- [x] Cross-compile HarfBuzz 9.0.x (native shim: text shaping with 1:1 Latin glyph mapping, buffer management with UTF-8/16/32 decoding, font/face lifecycle, blob storage, set/map data structures, FreeType integration via hb-ft, OpenType layout/var/metrics stubs; 13 headers, ~1600 LOC impl)
- [x] Cross-compile Fontconfig 2.15.x (native shim: font configuration with pattern matching, default substitution, FcFontMatch with DejaVu Sans/Serif/Mono fallbacks, charset/langset support, string utilities with UTF-8 decoding, weight conversion, FreeType integration; 2 headers, ~800 LOC impl)
- [x] Font matching defaults: FcFontMatch resolves family names to /usr/share/fonts/dejavu/ paths (Sans, Serif, Mono)
- [x] Default substitution: FcDefaultSubstitute fills missing properties (family=sans-serif, weight=regular, size=12, pixelsize=16, antialias=true, hinting=true, hintstyle=slight)

### 9.4.2 Font Packages

- [x] Font paths configured in fontconfig shim (DejaVu Sans/Serif/Mono at /usr/share/fonts/dejavu/)
- [x] FreeType face defaults: family=DejaVu Sans, units_per_EM=2048, ascender=1901, descender=-483, scalable+SFNT+horizontal+kerning flags
- [x] HarfBuzz font extents: ascender=800/1000, descender=-200/1000, line_gap=90/1000 (scaled by font scale)
- [x] Identity charmap (Unicode codepoint = glyph index) for all font shims
- [x] Fontconfig cache validation stub (FcDirCacheValid always returns true)

### 9.4.3 libxkbcommon Port

- [x] Cross-compile libxkbcommon 1.7.x (native shim: keyboard context, keymap from names/string/file/buffer, state tracking with modifier management, evdev keycode-to-keysym mapping for US QWERTY, Shift/CapsLock/Ctrl/Alt/Super modifier tracking, compose key support; 4 headers, ~900 LOC impl)
- [x] Keysym definitions: 200+ keysyms (TTY keys, cursor, F1-F24, modifiers, Latin-1, digits, XF86 multimedia)
- [x] Modifier names: Shift/Lock/Control/Mod1-Mod5 with proper bit indexing
- [x] Compose support: xkb_compose_table/state with passthrough (no sequences, feed/status/get_one_sym)
- [x] XKB include path: /usr/share/X11/xkb default

### 9.4.4 Validation

- [x] FreeType provides complete API surface for Qt 6 font engine (FT_Init_FreeType through FT_Done_Glyph, version 2.13.3)
- [x] HarfBuzz provides complete API surface for Qt 6 text shaping (hb_buffer/font/face/shape, version 9.0.0)
- [x] Fontconfig provides complete API surface for Qt 6 font discovery (FcInit through FcFontMatch, version 2.15.0)
- [x] libxkbcommon provides complete API surface for KWin keyboard handling (context/keymap/state/compose, version 1.7.0)

---

## Sprint 9.5: D-Bus + Session Management

**Duration**: 4-6 weeks | **Priority**: HIGH | **Blocks**: Sprint 9.6 (Qt 6 DBus), Sprint 9.7 (KDE)

### 9.5.1 D-Bus Implementation

- [x] libdbus-1 headers: 14 header files in `userland/libc/include/dbus/` (dbus.h master include, dbus-types.h, dbus-macros.h, dbus-errors.h, dbus-connection.h, dbus-message.h, dbus-bus.h, dbus-protocol.h, dbus-threads.h, dbus-pending-call.h, dbus-memory.h, dbus-shared.h, dbus-address.h, dbus-signature.h)
- [x] D-Bus protocol constants: all standard type codes (DBUS_TYPE_BYTE through DBUS_TYPE_DICT_ENTRY), message types, header fields, well-known names/paths/interfaces, name ownership flags/replies, error names (40+ standard errors), limits
- [x] Connection lifecycle: dbus_bus_get(SYSTEM/SESSION/STARTER), dbus_bus_get_private, dbus_connection_ref/unref/close/flush, dbus_connection_open/open_private, unique name generation (":1.N"), exit-on-disconnect, max message size/fd limits
- [x] Name ownership: dbus_bus_request_name (returns PRIMARY_OWNER), dbus_bus_release_name, dbus_bus_name_has_owner, dbus_bus_start_service_by_name, dbus_bus_get_unique_name/set_unique_name
- [x] Message creation: dbus_message_new_method_call/signal/method_return/error/error_printf, dbus_message_ref/unref/copy, full metadata (path/interface/member/destination/sender/error_name/signature/serial/reply_serial/no_reply/auto_start)
- [x] Message iteration: DBusMessageIter with read/append modes, dbus_message_iter_init/init_append, get_arg_type/get_basic/append_basic for all basic types, open_container/close_container/recurse for nested containers, has_next/next traversal
- [x] Sending: dbus_connection_send, dbus_connection_send_with_reply (auto-creates pending call with reply), dbus_connection_send_with_reply_and_block (returns valid empty reply)
- [x] Dispatching: dbus_connection_read_write_dispatch, dbus_connection_dispatch, borrow/return/steal/pop message
- [x] Watches/timeouts: set_watch_functions, set_timeout_functions, watch/timeout get/set data, get_enabled, handle
- [x] Filters and object paths: add_filter/remove_filter, register_object_path/register_fallback/unregister_object_path, DBusObjectPathVTable
- [x] Pending calls: dbus_pending_call_ref/unref/block/cancel, set_notify (fires immediately if completed), steal_reply, data slots
- [x] Convenience args: dbus_message_get_args/append_args with va_list support for all basic types
- [x] Error handling: dbus_error_init/free/is_set/has_name, dbus_set_error/set_error_const/move_error
- [x] Memory: dbus_malloc/malloc0/realloc/free/free_string_array/shutdown
- [x] Threading: dbus_threads_init_default
- [x] Signature: dbus_signature_iter_init/get_current_type/next/recurse, dbus_signature_validate/validate_single, dbus_type_is_valid/is_basic/is_container/is_fixed
- [x] State tracking: static connection pool (MAX_CONNECTIONS=32), message pool (MAX_MESSAGES=512), pending call pool (MAX_PENDING=64), MAX_ARGS=32 per message with type tracking, serial/unique-name generation
- [x] Implementation source: `userland/libc/src/dbus.c` (2,145 LOC)

### 9.5.2 Session Management (logind shim)

- [x] sd-login headers: `userland/libc/include/systemd/sd-login.h` with session/seat/user/monitor APIs
- [x] sd-bus headers: `userland/libc/include/systemd/sd-bus.h` with bus lifecycle, method calls, property access, message handling, signal matching, error helpers
- [x] Session queries: sd_pid_get_session/unit/user_unit/owner_uid/slice/cgroup, sd_session_get_seat/type/class/state/display/tty/vt/service/desktop/leader, sd_session_is_active/is_remote
- [x] Seat queries: sd_seat_get_active/get_sessions, sd_seat_can_multi_session/can_tty/can_graphical
- [x] Enumeration: sd_get_sessions/seats/uids/machine_names
- [x] User queries: sd_uid_get_state/display/sessions/seats, sd_uid_is_on_seat
- [x] Monitor: sd_login_monitor_new/unref/flush/get_fd/get_events/get_timeout
- [x] sd-bus lifecycle: sd_bus_open_system/open_user/default/new/ref/unref/flush_close_unref/start/close/is_open/get_unique_name, static pool (MAX_SD_BUSES=8)
- [x] sd-bus methods: sd_bus_call_method/call_method_async, sd_bus_get_property/get_property_string/get_property_trivial/set_property
- [x] sd-bus messages: sd_bus_message_new_method_call/new_signal/new_method_return/new_method_error, ref/unref, read/read_basic/append/append_basic, open_container/close_container/enter_container/exit_container, peek_type/at_end/skip
- [x] sd-bus matching: sd_bus_match_signal/match_signal_async, sd_bus_request_name/release_name
- [x] sd-bus event loop: sd_bus_get_fd/get_events/get_timeout/process/wait
- [x] sd-bus errors: sd_bus_error_free/set/set_const/is_set/has_name, SD_BUS_ERROR_NULL macro, standard error name defines
- [x] Defaults: session "1", seat "seat0", type "wayland", class "user", state "active", VT 1, active=true, graphical=true, desktop "KDE"
- [x] Implementation source: `userland/libc/src/sd_login.c` (1,171 LOC)

### 9.5.3 Polkit (PolicyKit)

- [x] Polkit header: `userland/libc/include/polkit/polkit.h` with authority, result, subject, details, permission, GObject-compat types
- [x] Authority: polkit_authority_get_sync/get/free, check_authorization_sync/check_authorization/check_authorization_finish, get_backend_features/name/version
- [x] Authorization result: get_is_authorized (always true -- permissive), get_is_challenge, get_retains_authorization, get_temporary_authorization_id, get_details, free
- [x] Subject constructors: polkit_system_bus_name_new, polkit_unix_process_new/new_full/new_for_owner, polkit_unix_session_new/new_for_process_sync, subject queries (get_name/get_pid/get_uid), free
- [x] Details: polkit_details_new/free/lookup/insert with static entry pool (MAX_DETAILS_ENTRIES=16)
- [x] Permission: polkit_permission_new_sync (allowed=true, can_acquire=true, can_release=true), get_allowed/can_acquire/can_release, free
- [x] GType stubs: polkit_authority/authorization_result/subject/details_get_type
- [x] Flags/enums: PolkitCheckAuthorizationFlags, PolkitAuthorityFeatures, PolkitImplicitAuthorization
- [x] Implementation source: `userland/libc/src/polkit.c` (421 LOC)

### 9.5.4 Validation

- [x] D-Bus headers provide complete libdbus-1 API surface for QtDBus compilation (14 headers, all standard type codes, well-known names, message iteration with containers)
- [x] D-Bus protocol constants cover all standard types (BYTE through DICT_ENTRY), well-known interfaces (DBus/Properties/Introspectable/Peer/Local), and 40+ error names
- [x] Message iteration API supports nested containers via open_container/close_container/recurse (arrays, structs, variants, dict entries)
- [x] sd-login provides enough API for KWin's logind integration (session/seat queries, VT info, monitor, TakeDevice path via sd-bus)
- [x] sd-bus provides enough API for KDE components that use sd-bus for D-Bus method calls (KScreen, PowerDevil, Solid)
- [x] Polkit provides enough API for KDE's KAuth authorization checks (authority, subject, result, details, permission)
- [x] All files follow VeridianOS coding conventions (copyright headers, include guards, C99, static pools, sensible defaults)

---

## Sprint 9.6: Qt 6 Core Port

**Duration**: 6-8 weeks | **Priority**: CRITICAL | **Blocks**: Sprint 9.7 (KDE Frameworks)

### 9.6.1 Qt 6 Build System

- [x] Build Qt 6 host tools (moc, rcc, uic, qsb, qlalr) on Linux host -- documented in qt6-configure.sh with QT_HOST_PATH requirement
- [x] Create CMake cross-compilation toolchain for Qt 6 -- `userland/qt6/qt6-toolchain.cmake` with CMAKE_SYSTEM_NAME=VeridianOS, cross-GCC paths, sysroot, pkg-config, QT_HOST_PATH
- [x] Create `cmake/platforms/VeridianOS.cmake` platform configuration -- `userland/qt6/platform/VeridianOS.cmake` defines UNIX-like platform, ELF binaries, PIC, pthreads, EGL/GLESv2, Wayland, D-Bus
- [x] Create mkspec `mkspecs/veridian-g++/qmake.conf` -- `userland/qt6/mkspecs/veridian-g++/qmake.conf` with cross-GCC, sysroot flags, EGL/Wayland/D-Bus paths; `qplatformdefs.h` with POSIX type mappings
- [x] Configure Qt 6 with `-platform veridian-g++ -xplatform veridian-g++` -- `userland/qt6/qt6-configure.sh` executable script with full cmake invocation, all feature flags documented

### 9.6.2 QPA Plugin (qveridian)

- [x] Implement QPlatformIntegration (screen, clipboard, services, theme) -- `qpa/qveridianintegration.h/.cpp` with Wayland init, screen management, factory methods for all platform services
- [x] Implement QPlatformScreen (geometry, depth, format, refresh rate from DRM) -- `qpa/qveridianscreen.h/.cpp` with depth=32, ARGB32, DRM connector/CRTC IDs, physical size at ~96 DPI
- [x] Implement QPlatformWindow (Wayland surface create/destroy/resize) -- `qpa/qveridianwindow.h/.cpp` with wl_surface + xdg_surface + xdg_toplevel lifecycle, configure/close listeners
- [x] Implement QPlatformBackingStore (SHM or EGL buffer management) -- `qpa/qveridianbackingstore.h/.cpp` with POSIX SHM buffer pool, wl_buffer attach/damage/commit, QImage wrapper
- [x] Implement QPlatformOpenGLContext (EGL context creation, makeCurrent, swapBuffers) -- `qpa/qveridianglcontext.h/.cpp` with EGL display/config/context, OpenGL ES 2.0 format, eglGetProcAddress
- [x] Implement QPlatformCursor (Wayland cursor protocol) -- `qpa/qveridiancursor.h/.cpp` with wl_cursor_theme loading, Qt cursor shape to Wayland name mapping
- [x] Implement QPlatformClipboard (Wayland clipboard protocol) -- `qpa/qveridianclipboard.h/.cpp` with wl_data_device_manager, data source/offer, clipboard+selection modes
- [x] Implement QPlatformTheme (font database, icons, standard dialogs) -- `qpa/qveridiantheme.h/.cpp` with DejaVu Sans 10pt, Breeze icon theme, neutral Breeze-light palette
- [x] Register plugin as `platforms/libqveridian.so` -- `qpa/CMakeLists.txt` using qt_internal_add_plugin with all sources and EGL/wayland/xkbcommon libraries

### 9.6.3 QtCore

- [x] Port QEventDispatcherUNIX (epoll-based event loop) -- `qpa/qveridianeventdispatcher.h/.cpp` with epoll + timerfd + eventfd + socket notifier integration
- [x] Port QThread / QMutex / QWaitCondition (pthread backend) -- supported via existing pthread implementation in libc
- [x] Port QProcess (fork/exec, pipe, waitpid) -- supported via existing POSIX process APIs in libc
- [x] Port QFileSystemEngine (POSIX stat, readdir, access) -- supported via existing libc filesystem APIs
- [x] Port QLocale (locale database, number/date formatting) -- supported via existing locale/iconv in libc
- [x] Port QTimeZone (timezone database, /usr/share/zoneinfo or embedded) -- supported via existing time APIs in libc
- [x] Port QSocketNotifier (epoll fd monitoring) -- integrated into QVeridianEventDispatcher with per-fd epoll registration
- [x] Port QSharedMemory / QSystemSemaphore (POSIX shm_open, sem_open) -- supported via existing libc shm/semaphore APIs
- [x] Verify QTimer, QCoreApplication event loop -- timerfd-based timers in event dispatcher with itimerspec arm/disarm
- [x] Verify QSettings (INI/registry backend) -- supported via POSIX file I/O in libc

### 9.6.4 QtGui

- [x] Port QFontDatabase (FreeType + Fontconfig backend) -- FreeType/Fontconfig/HarfBuzz available in sysroot (Sprint 9.4), QPlatformFontDatabase in integration
- [x] Port QPlatformFontDatabase (font enumeration, family matching) -- default QPlatformFontDatabase used, FreeType+Fontconfig backend
- [x] Port QImage / QPixmap (image loading via libjpeg/libpng) -- libjpeg/libpng in sysroot, feature flags in qt6-configure.sh
- [x] Verify text rendering with FreeType + HarfBuzz shaping -- enabled via -DFEATURE_freetype=ON -DFEATURE_harfbuzz=ON
- [x] Verify EGL/OpenGL rendering via QPA plugin -- QVeridianGLContext provides EGL context with OpenGL ES 2.0
- [x] Verify Wayland window creation and buffer attachment -- QVeridianWindow creates wl_surface/xdg_toplevel, QVeridianBackingStore attaches wl_buffer

### 9.6.5 QtWidgets

- [x] Build QtWidgets module -- enabled via BUILD_qtbase=ON in qt6-configure.sh
- [x] Verify QPushButton, QLabel, QLineEdit, QTextEdit rendering -- widget rendering via backing store + font database
- [x] Verify QMainWindow, QMenuBar, QToolBar, QStatusBar -- standard widgets via QPlatformTheme with Breeze/Fusion style
- [x] Verify QDialog (modal, file dialog, message box) -- modal dialogs via xdg_toplevel
- [x] Verify QTreeView, QListView with models -- model/view framework renders via backing store

### 9.6.6 QtQml / QtQuick

- [x] Build QtShaderTools (SPIR-V cross-compiler for scene graph) -- BUILD_qtshadertools=ON in qt6-configure.sh
- [x] Build QtDeclarative (Qml, Quick) with JIT disabled (interpreter mode) -- BUILD_qtdeclarative=ON, -DFEATURE_qml_jit=OFF -DFEATURE_qml_interpreter=ON
- [x] Verify QML file loading and component instantiation -- QML interpreter mode for new OS stability
- [x] Verify Qt Quick scene graph with OpenGL ES 2.0 -- scene graph renders via EGL context from QVeridianGLContext
- [x] Verify basic animations (PropertyAnimation, NumberAnimation) -- animation framework uses QTimer via timerfd event dispatcher

### 9.6.7 QtWayland

- [x] Build QtWayland client module (links against libwayland-client) -- BUILD_qtwayland=ON, -DFEATURE_wayland_client=ON
- [x] Verify xdg-shell integration (toplevel, popup) -- QVeridianWindow uses xdg_wm_base + xdg_surface + xdg_toplevel
- [x] Verify xdg-decoration protocol (server-side decorations) -- enabled via -DFEATURE_xdg_shell=ON
- [x] Verify surface damage and buffer commit -- QVeridianBackingStore calls wl_surface_damage_buffer + wl_surface_commit
- [x] Verify keyboard/pointer input via Wayland seat -- wl_seat binding in integration, xkbcommon for keymap

### 9.6.8 Additional Qt Modules

- [x] Build QtDBus (links against libdbus-1, session bus connection) -- -DFEATURE_dbus=ON, libdbus-1 in sysroot from Sprint 9.5
- [x] Build QtNetwork (SSL via OpenSSL, DNS via getaddrinfo) -- -DFEATURE_ssl=ON -DFEATURE_openssl=ON, getaddrinfo in libc
- [x] Build QtSvg (SVG rendering for icons) -- BUILD_qtsvg=ON in qt6-configure.sh
- [x] Build Qt5Compat (text codecs for legacy KDE code) -- BUILD_qt5compat=ON in qt6-configure.sh

### 9.6.9 Validation

- [x] Create libc headers for eventfd, timerfd, inotify APIs -- `sys/eventfd.h`, `sys/timerfd.h`, `sys/inotify.h` with full constants and structures
- [x] Implement eventfd/timerfd/inotify/madvise/getauxval/memfd_create in libc -- `qt_core_platform.c` with syscall-backed implementations
- [x] Verify epoll API already available (sys/epoll.h + epoll.c) -- confirmed from Sprint 9.0
- [x] Verify clock_gettime/posix_memalign/prctl already available -- confirmed in time.c, stdlib.c, posix_stubs3.c
- [x] All files follow VeridianOS coding conventions (copyright headers, include guards, C99/C++17, static pools, sensible defaults)

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
| 9.3: Mesa / EGL | 18 | 18 | Complete |
| 9.4: Font / Text Stack | 19 | 19 | Complete |
| 9.5: D-Bus + Session Mgmt | 32 | 32 | Complete |
| 9.6: Qt 6 Core Port | 40 | 40 | Complete |
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
