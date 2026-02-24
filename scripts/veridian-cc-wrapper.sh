#!/bin/bash
# veridian-cc-wrapper.sh -- CC wrapper for BusyBox cross-compilation
#
# Wraps x86_64-veridian-gcc to inject VeridianOS-specific flags for:
#   - Compilation: -nostdinc + sysroot include paths
#   - Linking: CRT objects + library paths + -lc -lgcc
#
# BusyBox Makefile calls $(CC) for both compile and link steps.
# We detect which mode by checking for '-c' (compile) or the
# presence of '-o busybox' / '.o' files (link).

TOOLCHAIN="/opt/veridian/toolchain"
CC="${TOOLCHAIN}/bin/x86_64-veridian-gcc"
SYSROOT="${TOOLCHAIN}/sysroot"
GCC_LIBDIR="${TOOLCHAIN}/lib/gcc/x86_64-veridian/14.2.0"
GCC_INCDIR="${GCC_LIBDIR}/include"

# CRT objects for linking
CRT0="${SYSROOT}/usr/lib/crt0.o"
CRTI="${SYSROOT}/usr/lib/crti.o"
CRTN="${SYSROOT}/usr/lib/crtn.o"
CRTBEGIN="${GCC_LIBDIR}/crtbegin.o"
CRTEND="${GCC_LIBDIR}/crtend.o"

# Detect if this is a link step or compile step
IS_LINK=0
IS_COMPILE=0
HAS_NOSTDINC=0
HAS_NOSTDLIB=0
OUTPUT_FILE=""

for arg in "$@"; do
    case "$arg" in
        -c) IS_COMPILE=1 ;;
        -E) IS_COMPILE=1 ;;  # Preprocessing only
        -S) IS_COMPILE=1 ;;  # Assembly only
        -nostdinc) HAS_NOSTDINC=1 ;;
        -nostdlib) HAS_NOSTDLIB=1 ;;
    esac
done

# If not a compile step (-c/-E/-S), and there are .o files in args, it's a link step
if [ "$IS_COMPILE" = "0" ]; then
    for arg in "$@"; do
        case "$arg" in
            *.o) IS_LINK=1; break ;;
        esac
    done
fi

if [ "$IS_COMPILE" = "1" ]; then
    # ---- Compile step ----
    exec "$CC" \
        -nostdinc \
        -isystem "${SYSROOT}/usr/include" \
        -isystem "${GCC_INCDIR}" \
        -static \
        -fno-stack-protector \
        -ffreestanding \
        -mno-red-zone \
        -mcmodel=small \
        -D__veridian__ \
        -D__linux__ \
        -Wno-unused-parameter \
        -Wno-implicit-function-declaration \
        -Wno-return-type \
        "$@"
elif [ "$IS_LINK" = "1" ]; then
    # ---- Link step ----
    # We inject CRT objects and libraries around BusyBox's .o and .a files.

    # Collect all args, preserving order (including .o, .a, -Wl, etc.)
    # We inject -lc -lgcc just before each --end-group so the linker
    # can resolve circular dependencies between BusyBox's main() and
    # our libc's __libc_start_main().
    LINK_ARGS=()

    while [ $# -gt 0 ]; do
        case "$1" in
            -static|-nostdlib|-nostdinc)
                # We handle these ourselves
                shift
                ;;
            -lm)
                # libm -- skip, our libc includes math stubs
                shift
                ;;
            -Wl,--end-group)
                # Inject -lc -lgcc inside the group before closing it
                LINK_ARGS+=("-L${SYSROOT}/usr/lib" "-L${GCC_LIBDIR}" "-lc" "-lgcc" "$1")
                shift
                ;;
            *)
                LINK_ARGS+=("$1")
                shift
                ;;
        esac
    done

    # Two modes:
    #
    # 1. Caller passed -nostdlib (e.g. BusyBox):
    #    The caller provides its own _start/_init/_fini in its .o files.
    #    We must NOT inject our CRT objects (crt0.o, crti.o, etc.) or they
    #    will conflict.  We only inject -lc -lgcc (already done above).
    #
    # 2. Normal programs (no -nostdlib):
    #    We provide the full CRT startup sequence:
    #      crt0.o (_start -> __libc_start_main -> main)
    #      crti.o / crtn.o (.init/.fini sections)
    #      crtbegin.o / crtend.o (C++ static constructors/destructors)
    if [ "$HAS_NOSTDLIB" = "1" ]; then
        exec "$CC" \
            -static \
            -nostdlib \
            -ffreestanding \
            "${LINK_ARGS[@]}"
    else
        exec "$CC" \
            -static \
            -nostdlib \
            -ffreestanding \
            "$CRT0" "$CRTI" "$CRTBEGIN" \
            "${LINK_ARGS[@]}" \
            "$CRTEND" "$CRTN"
    fi
else
    # ---- Fallback: pass through with basic flags ----
    exec "$CC" \
        -nostdinc \
        -isystem "${SYSROOT}/usr/include" \
        -isystem "${GCC_INCDIR}" \
        -static \
        -fno-stack-protector \
        -ffreestanding \
        -D__veridian__ \
        -Wno-implicit-function-declaration \
        "$@"
fi
