/*
 * VeridianOS libc -- <sys/sysmacros.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Device number manipulation macros.
 * VeridianOS uses a 64-bit dev_t with 32-bit major/minor split.
 */

#ifndef _SYS_SYSMACROS_H
#define _SYS_SYSMACROS_H

#ifdef __cplusplus
extern "C" {
#endif

#define major(dev)      ((unsigned int)(((dev) >> 32) & 0xFFFFFFFF))
#define minor(dev)      ((unsigned int)((dev) & 0xFFFFFFFF))
#define makedev(ma, mi) ((dev_t)(((dev_t)(ma) << 32) | ((dev_t)(mi) & 0xFFFFFFFF)))

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SYSMACROS_H */
