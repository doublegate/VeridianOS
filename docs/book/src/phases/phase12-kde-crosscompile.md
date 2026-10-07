# Phase 12: KDE Plasma 6 Cross-Compilation

**Version**: v0.25.0 | **Date**: March 2026 | **Status**: COMPLETE

## Overview

Phase 12 cross-compiles the entire KDE Plasma 6 stack from source, producing statically
linked x86-64 ELF binaries that run on VeridianOS without a dynamic linker. A 10-phase
musl-based build pipeline compiles over 60 upstream projects -- from the C library through
Qt 6, KDE Frameworks, and Plasma -- into three self-contained binaries packaged in a
BlockFS root filesystem image.

## Build Pipeline

The versions below are the ones the completed v0.25.0 build used. Since 2026-10-07 the
scripts in `tools/cross/` pin the latest stable releases instead, and the stack has not
been rebuilt with them yet (see "Pinned versions" below).

1. **musl 1.2.5** -- C library (static libc.a)
2. **8 C dependencies** -- zlib, libpng, libjpeg (SIMD off), libffi, pcre2, libxml2,
   libxslt, libudev-zero
3. **Mesa 24.2.8** -- Software rasterizer (softpipe), static archives via `ar -M` MRI
   extraction from `.so.p` object directories
4. **Wayland 1.23.1** -- Client and server protocol libraries
5. **FreeType / HarfBuzz / Fontconfig** -- Font rendering stack
6. **D-Bus 1.14.10** -- Message bus daemon
7. **Qt 6.8.3** -- 12 modules (Core, Gui, Widgets, Network, DBus, Xml, QML, Quick,
   WaylandClient, WaylandCompositor, Svg, ShaderTools)
8. **KDE Frameworks 6.12.0** -- 35+ modules with `MODULE` to `STATIC` sed patches
9. **KWin 6.3.5** -- Wayland compositor
10. **Plasma 6.3.5** -- 9 shell components

### Pinned versions (scripts, 2026-10-07; not yet built)

| Component | Built (v0.25.0) | Pinned now |
|---|---|---|
| musl | 1.2.5 | 1.2.6 |
| Mesa / libdrm | 24.2.8 / 2.4.123 | 26.2.4 / 2.4.134 |
| Wayland / wayland-protocols | 1.23.1 / 1.38 | 1.26.0 / 1.49 |
| FreeType / HarfBuzz / Fontconfig | 2.13.2 / 8.3.0 / 2.15.0 | 2.14.3 / 14.6.0 / 2.18.3 |
| D-Bus | 1.14.10 (autotools) | 1.16.2 (meson; 1.16 dropped autotools) |
| Qt | 6.8.3 | 6.12.0 (WaylandClient now comes from qtbase) |
| KDE Frameworks / plasma-wayland-protocols | 6.12.0 / 1.15.0 | 6.30.0 / 1.23.0 |
| KWin / KDecoration / Plasma | 6.3.5 | 6.7.5 |
| zlib, libffi, pcre2, expat, libxml2 | 1.3.1, 3.4.6, 10.43, 2.6.2, 2.12.6 | 1.3.2, 3.8.0, 10.49, 2.9.0, 2.15.4 |
| libjpeg-turbo, libpng, libxkbcommon | 3.0.3, 1.6.43, 1.7.0 | 3.2.0, 1.6.59, 1.13.2 |
| SQLite, OpenSSL | 3.49.1, 3.3.2 | 3.53.4, 4.0.3 |
| libevdev, libinput, at-spi2-core | 1.13.3, 1.26.2, 2.52.0 | 1.14.0, 1.32.0, 2.62.0 |

Checked without building: every download URL resolves to a real archive, the wayland patch
applies with no fuzz, every meson `-D` option and Qt configure flag and feature exists in the
new sources (removed Mesa and at-spi2-core options were dropped or renamed), and every Qt
submodule, KDE Frameworks module and Plasma component fetched exists in the new releases.
Not checked until a build runs: source patches applied with `sed`/`python` in the scripts,
the static-archive extraction from Mesa's `.so.p` directories, and the VeridianOS Qt
platform plugin against Qt 6.12's private API.

## Key Deliverables

- **kwin_wayland**: 158 MB raw / 64 MB stripped static ELF binary
- **plasmashell**: 150 MB raw / 59 MB stripped static ELF binary
- **dbus-daemon**: 886 KB static ELF binary
- **Sysroot**: 250+ static `.a` archives totaling ~1.1 GB
- **Root filesystem**: 479 MB BlockFS image (245 inodes, 22 fonts, 3 binaries,
  D-Bus/XDG/session configuration files)
- **Build scripts**: 15 shell scripts in `tools/cross/`, 2 CMake toolchain files,
  1 musl syscall compatibility patch

## Technical Highlights

- **Mesa static archives**: Mesa hardcodes `shared_library()` in Meson. Workaround
  extracts `.o` files from `.so.p` build directories and creates fat `.a` archives
  using `ar -M` MRI scripts, then rewrites pkg-config files
- **libjpeg SIMD/TLS fix**: SIMD-enabled libjpeg generates `R_X86_64_TPOFF32`
  relocations incompatible with static PIE. Disabled SIMD and added `-fPIC`
- **Qt 6 host+cross split**: Host Qt build requires `-gui -widgets` for tool
  generation (qmlcachegen). Cross build uses `-k || true` (tools fail, libraries
  succeed)
- **KDE MODULE to STATIC**: Sed patches rewrite `add_library(... MODULE` to `STATIC`
  at build time, converting all KDE plugins to static linkage
- **CMAKE_SYSROOT disabled**: musl-g++ manages include paths via `-nostdinc`/`-isystem`;
  CMake's `--sysroot=` flag conflicts with this ordering
- **GL/KF6/udev stub libraries**: Minimal `.a` stubs satisfy link-time dependencies
  for subsystems not yet available on VeridianOS
- **glibc_shim**: Compatibility shim for GCC 15's libstdc++ when building against musl
- **C++ udev mangling**: Qt 6 compiles udev headers without `extern "C"`, expecting
  C++ mangled symbols. Solution: build libudev.a as C++ to match

## Files and Statistics

- Build phases: 10
- Upstream projects compiled: 60+
- Build scripts: 15 (in `tools/cross/`)
- Output binaries: 3 static ELF executables
- Static archives: 250+
- BlockFS image: 479 MB (245 inodes)
- Commits: 7
