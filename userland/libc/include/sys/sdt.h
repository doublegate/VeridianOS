/*
 * VeridianOS sys/sdt.h -- SystemTap/DTrace probe stubs
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stub header providing no-op probe macros for software that
 * conditionally uses SystemTap probes (e.g., libgcc unwinder).
 */

#ifndef _SYS_SDT_H
#define _SYS_SDT_H

/* No-op probe macros -- all probes are compiled out */
#define STAP_PROBE(provider, name)
#define STAP_PROBE1(provider, name, a1)
#define STAP_PROBE2(provider, name, a1, a2)
#define STAP_PROBE3(provider, name, a1, a2, a3)
#define STAP_PROBE4(provider, name, a1, a2, a3, a4)
#define STAP_PROBE5(provider, name, a1, a2, a3, a4, a5)
#define STAP_PROBE6(provider, name, a1, a2, a3, a4, a5, a6)
#define STAP_PROBE7(provider, name, a1, a2, a3, a4, a5, a6, a7)
#define STAP_PROBE8(provider, name, a1, a2, a3, a4, a5, a6, a7, a8)
#define STAP_PROBE9(provider, name, a1, a2, a3, a4, a5, a6, a7, a8, a9)
#define STAP_PROBE10(provider, name, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10)

/* DTRACE compatibility */
#define DTRACE_PROBE(provider, name)
#define DTRACE_PROBE1(provider, name, a1)
#define DTRACE_PROBE2(provider, name, a1, a2)
#define DTRACE_PROBE3(provider, name, a1, a2, a3)
#define DTRACE_PROBE4(provider, name, a1, a2, a3, a4)

#endif /* _SYS_SDT_H */
