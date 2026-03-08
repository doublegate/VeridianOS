/*
 * VeridianOS libc -- expat_external.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * External definitions for Expat XML parser.
 */

#ifndef _EXPAT_EXTERNAL_H
#define _EXPAT_EXTERNAL_H

#define XML_DTD          1
#define XML_NS           1
#define XML_CONTEXT_BYTES 1024
#define XML_STATIC       1

#ifndef XMLCALL
#define XMLCALL
#endif

#ifndef XMLIMPORT
#define XMLIMPORT
#endif

#define XMLPARSEAPI(type) type

#endif /* _EXPAT_EXTERNAL_H */
