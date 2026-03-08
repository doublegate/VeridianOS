/*
 * VeridianOS libc -- libxml/xmlstring.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 string types and operations.
 */

#ifndef _LIBXML_XMLSTRING_H
#define _LIBXML_XMLSTRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/** xmlChar is an unsigned char (UTF-8 byte). */
typedef unsigned char xmlChar;

/** Cast a string to xmlChar *. */
#define BAD_CAST (xmlChar *)

/** Get the length of an xmlChar string. */
int xmlStrlen(const xmlChar *str);

/** Compare two xmlChar strings. */
int xmlStrcmp(const xmlChar *str1, const xmlChar *str2);

/** Compare two xmlChar strings (case-insensitive). */
int xmlStrcasecmp(const xmlChar *str1, const xmlChar *str2);

/** Compare n bytes of two xmlChar strings. */
int xmlStrncmp(const xmlChar *str1, const xmlChar *str2, int len);

/** Duplicate an xmlChar string. */
xmlChar *xmlStrdup(const xmlChar *cur);

/** Duplicate n bytes of an xmlChar string. */
xmlChar *xmlStrndup(const xmlChar *cur, int len);

/** Concatenate xmlChar strings. */
xmlChar *xmlStrcat(xmlChar *cur, const xmlChar *add);

/** Find a substring. */
const xmlChar *xmlStrstr(const xmlChar *str, const xmlChar *val);

/** Find a character. */
const xmlChar *xmlStrchr(const xmlChar *str, xmlChar val);

/** Convert xmlChar string to integer. */
int xmlStrPrintf(xmlChar *buf, int len, const char *msg, ...);

#ifdef __cplusplus
}
#endif

#endif /* _LIBXML_XMLSTRING_H */
