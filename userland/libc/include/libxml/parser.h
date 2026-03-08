/*
 * VeridianOS libc -- libxml/parser.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 XML parser API.
 */

#ifndef _LIBXML_PARSER_H
#define _LIBXML_PARSER_H

#include "tree.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Parser options                                                            */
/* ========================================================================= */

typedef enum {
    XML_PARSE_RECOVER    = 1 << 0,
    XML_PARSE_NOENT      = 1 << 1,
    XML_PARSE_DTDLOAD    = 1 << 2,
    XML_PARSE_DTDATTR    = 1 << 3,
    XML_PARSE_DTDVALID   = 1 << 4,
    XML_PARSE_NOERROR    = 1 << 5,
    XML_PARSE_NOWARNING  = 1 << 6,
    XML_PARSE_PEDANTIC   = 1 << 7,
    XML_PARSE_NOBLANKS   = 1 << 8,
    XML_PARSE_XINCLUDE   = 1 << 10,
    XML_PARSE_NONET      = 1 << 11,
    XML_PARSE_NODICT     = 1 << 12,
    XML_PARSE_NSCLEAN    = 1 << 13,
    XML_PARSE_NOCDATA    = 1 << 14,
    XML_PARSE_NOXINCNODE = 1 << 15,
    XML_PARSE_COMPACT    = 1 << 16,
    XML_PARSE_HUGE       = 1 << 19,
    XML_PARSE_BIG_LINES  = 1 << 22
} xmlParserOption;

/* ========================================================================= */
/* Parser API                                                                */
/* ========================================================================= */

/** Parse an XML file and build a document tree. */
xmlDocPtr xmlParseFile(const char *filename);

/** Parse an XML buffer and build a document tree. */
xmlDocPtr xmlParseMemory(const char *buffer, int size);

/** Parse an XML buffer with options. */
xmlDocPtr xmlReadMemory(const char *buffer, int size,
                        const char *URL, const char *encoding,
                        int options);

/** Parse an XML file with options. */
xmlDocPtr xmlReadFile(const char *filename, const char *encoding,
                      int options);

/** Initialize the parser. */
void xmlInitParser(void);

/** Cleanup the parser. */
void xmlCleanupParser(void);

/** Get the last error. */
typedef struct _xmlError {
    int domain;
    int code;
    char *message;
    int level;
    char *file;
    int line;
    char *str1;
    char *str2;
    char *str3;
    int int1;
    int int2;
    void *ctxt;
    void *node;
} xmlError;

typedef xmlError *xmlErrorPtr;

/** Get the last parsing error. */
xmlErrorPtr xmlGetLastError(void);

/** Reset the last error. */
void xmlResetLastError(void);

/** Dump a document to a file descriptor. */
int xmlDocDump(void *f, xmlDocPtr cur);

/** Dump a document to a memory buffer. */
void xmlDocDumpMemory(xmlDocPtr cur, xmlChar **mem, int *size);

/** Dump a formatted document to a memory buffer. */
void xmlDocDumpFormatMemory(xmlDocPtr cur, xmlChar **mem, int *size,
                            int format);

#ifdef __cplusplus
}
#endif

#endif /* _LIBXML_PARSER_H */
