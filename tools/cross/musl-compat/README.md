# musl-compat: glibc compatibility layer for musl cross-compilation

When cross-compiling C++ code (like Mesa) using musl libc with a system GCC
whose libstdc++ was built against glibc, a handful of glibc-internal symbols
are missing. This directory provides compatibility shims.

## Files

- **glibc_shim.c** - Stub implementations for glibc symbols referenced by
  libstdc++.a: `__sprintf_chk`, `__fprintf_chk`, `__read_chk`,
  `__isoc23_strtoul`, `__libc_single_threaded`, `arc4random`,
  `_dl_find_object`, `__dso_handle`

- **glibc_compat.h** - Header included via `-include` in the C++ wrapper.
  Provides `__locale_t`, `__GLIBC_PREREQ`, `__THROW`, and inline stubs for
  `pthread_cond_clockwait` and `pthread_mutex_clocklock` (GCC 15 libstdc++
  header requirements not present in musl).

## Build

The shim is compiled and installed by `build-musl.sh` into the sysroot:

```bash
$CC -c -O2 -fPIC -o $SYSROOT/usr/lib/glibc_shim.o glibc_shim.c
ar rcs $SYSROOT/usr/lib/libglibc_shim.a $SYSROOT/usr/lib/glibc_shim.o
```

The musl-g++ wrapper (`$SYSROOT/bin/x86_64-veridian-musl-g++`) automatically
links against `-lglibc_shim` and uses `-include glibc_compat.h`.
