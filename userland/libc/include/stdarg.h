/*
 * VeridianOS libc -- <stdarg.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Variable argument list support.  GCC and Clang provide built-in
 * implementations; we just wrap them here so that programs that
 * #include <stdarg.h> get the compiler builtins.
 */

#ifndef _STDARG_H
#define _STDARG_H

/*
 * Use compiler builtins.  Both GCC and Clang define __GNUC__ and
 * provide __builtin_va_* intrinsics.
 */
#ifdef __GNUC__

typedef __builtin_va_list va_list;

#define va_start(ap, last)  __builtin_va_start(ap, last)
#define va_arg(ap, type)    __builtin_va_arg(ap, type)
#define va_end(ap)          __builtin_va_end(ap)
#define va_copy(dest, src)  __builtin_va_copy(dest, src)

#else
#error "VeridianOS libc <stdarg.h> requires GCC or Clang builtins"
#endif

#endif /* _STDARG_H */
