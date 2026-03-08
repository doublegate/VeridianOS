/*
 * VeridianOS libc -- expat.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Expat 2.6.x compatible XML parser API.
 * SAX-style event-driven XML parser.
 */

#ifndef _EXPAT_H
#define _EXPAT_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define XML_MAJOR_VERSION 2
#define XML_MINOR_VERSION 6
#define XML_MICRO_VERSION 0

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef char XML_Char;
typedef char XML_LChar;
typedef long XML_Index;
typedef unsigned long XML_Size;

/* Opaque parser handle */
typedef struct XML_ParserStruct *XML_Parser;

/* ========================================================================= */
/* Status / Error codes                                                      */
/* ========================================================================= */

enum XML_Status {
    XML_STATUS_ERROR    = 0,
    XML_STATUS_OK       = 1,
    XML_STATUS_SUSPENDED = 2
};

enum XML_Error {
    XML_ERROR_NONE                   = 0,
    XML_ERROR_NO_MEMORY              = 1,
    XML_ERROR_SYNTAX                 = 2,
    XML_ERROR_NO_ELEMENTS            = 3,
    XML_ERROR_INVALID_TOKEN          = 4,
    XML_ERROR_UNCLOSED_TOKEN         = 5,
    XML_ERROR_PARTIAL_CHAR           = 6,
    XML_ERROR_TAG_MISMATCH           = 7,
    XML_ERROR_DUPLICATE_ATTRIBUTE    = 8,
    XML_ERROR_JUNK_AFTER_DOC_ELEMENT = 9,
    XML_ERROR_PARAM_ENTITY_REF       = 10,
    XML_ERROR_UNDEFINED_ENTITY       = 11,
    XML_ERROR_RECURSIVE_ENTITY_REF   = 12,
    XML_ERROR_ASYNC_ENTITY           = 13,
    XML_ERROR_BAD_CHAR_REF           = 14,
    XML_ERROR_BINARY_ENTITY_REF      = 15,
    XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF = 16,
    XML_ERROR_MISPLACED_XML_PI       = 17,
    XML_ERROR_UNKNOWN_ENCODING       = 18,
    XML_ERROR_INCORRECT_ENCODING     = 19,
    XML_ERROR_UNCLOSED_CDATA_SECTION = 20,
    XML_ERROR_EXTERNAL_ENTITY_HANDLING = 21,
    XML_ERROR_NOT_STANDALONE         = 22,
    XML_ERROR_UNEXPECTED_STATE       = 23,
    XML_ERROR_ABORTED                = 24,
    XML_ERROR_FINISHED               = 25,
    XML_ERROR_SUSPEND_PE             = 26
};

/* ========================================================================= */
/* Handler typedefs                                                          */
/* ========================================================================= */

typedef void (*XML_StartElementHandler)(void *userData,
    const XML_Char *name, const XML_Char **atts);

typedef void (*XML_EndElementHandler)(void *userData,
    const XML_Char *name);

typedef void (*XML_CharacterDataHandler)(void *userData,
    const XML_Char *s, int len);

typedef void (*XML_CommentHandler)(void *userData,
    const XML_Char *data);

typedef void (*XML_ProcessingInstructionHandler)(void *userData,
    const XML_Char *target, const XML_Char *data);

typedef void (*XML_StartCdataSectionHandler)(void *userData);
typedef void (*XML_EndCdataSectionHandler)(void *userData);

typedef void (*XML_DefaultHandler)(void *userData,
    const XML_Char *s, int len);

typedef void (*XML_XmlDeclHandler)(void *userData,
    const XML_Char *version, const XML_Char *encoding, int standalone);

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

/** Create a new parser with the given encoding (or NULL for auto). */
XML_Parser XML_ParserCreate(const XML_Char *encoding);

/** Create a new parser with namespace processing. */
XML_Parser XML_ParserCreateNS(const XML_Char *encoding,
                               XML_Char namespaceSeparator);

/** Free the parser. */
void XML_ParserFree(XML_Parser parser);

/** Reset the parser for reuse. */
int XML_ParserReset(XML_Parser parser, const XML_Char *encoding);

/** Set the user data pointer. */
void XML_SetUserData(XML_Parser parser, void *userData);

/** Set element start/end handlers. */
void XML_SetElementHandler(XML_Parser parser,
                           XML_StartElementHandler start,
                           XML_EndElementHandler end);

/** Set character data handler. */
void XML_SetCharacterDataHandler(XML_Parser parser,
                                 XML_CharacterDataHandler handler);

/** Set comment handler. */
void XML_SetCommentHandler(XML_Parser parser,
                           XML_CommentHandler handler);

/** Set processing instruction handler. */
void XML_SetProcessingInstructionHandler(XML_Parser parser,
    XML_ProcessingInstructionHandler handler);

/** Set CDATA section handlers. */
void XML_SetCdataSectionHandler(XML_Parser parser,
                                XML_StartCdataSectionHandler start,
                                XML_EndCdataSectionHandler end);

/** Set default handler. */
void XML_SetDefaultHandler(XML_Parser parser,
                           XML_DefaultHandler handler);

/** Set XML declaration handler. */
void XML_SetXmlDeclHandler(XML_Parser parser,
                           XML_XmlDeclHandler handler);

/** Parse a chunk of XML data. isFinal=1 for the last chunk. */
enum XML_Status XML_Parse(XML_Parser parser, const char *s,
                          int len, int isFinal);

/** Get the error code from the last parse. */
enum XML_Error XML_GetErrorCode(XML_Parser parser);

/** Get a human-readable error string for an error code. */
const XML_LChar *XML_ErrorString(enum XML_Error code);

/** Get the current line number in the parse. */
XML_Size XML_GetCurrentLineNumber(XML_Parser parser);

/** Get the current column number in the parse. */
XML_Size XML_GetCurrentColumnNumber(XML_Parser parser);

/** Get the current byte index in the parse. */
XML_Index XML_GetCurrentByteIndex(XML_Parser parser);

/** Get the version string. */
const XML_LChar *XML_ExpatVersion(void);

#ifdef __cplusplus
}
#endif

#endif /* _EXPAT_H */
