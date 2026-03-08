/*
 * VeridianOS libc -- expat_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Expat 2.6.x compatible SAX-style XML parser.
 * Implements a state-machine parser that handles elements, attributes,
 * character data, comments, CDATA sections, and processing instructions.
 *
 * Parser states: OUTSIDE -> TAG_OPEN -> TAG_NAME -> ATTR_NAME ->
 *                ATTR_EQ -> ATTR_VAL -> TAG_BODY -> TEXT
 */

#include <expat.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal parser state                                                     */
/* ========================================================================= */

#define MAX_ATTR      64  /* max attributes per element */
#define MAX_DEPTH     256 /* max nesting depth */
#define BUF_SIZE      4096

enum parse_state {
    PS_OUTSIDE,         /* between tags */
    PS_TAG_OPEN,        /* saw '<' */
    PS_TAG_NAME,        /* reading element name */
    PS_CLOSE_TAG_NAME,  /* reading </name */
    PS_ATTR_SPACE,      /* between attributes */
    PS_ATTR_NAME,       /* reading attribute name */
    PS_ATTR_EQ,         /* saw attribute name, expecting '=' */
    PS_ATTR_VAL_START,  /* saw '=', expecting quote */
    PS_ATTR_VAL,        /* reading attribute value */
    PS_TAG_CLOSE,       /* saw '/' in self-closing */
    PS_COMMENT,         /* inside <!-- ... --> */
    PS_CDATA,           /* inside <![CDATA[ ... ]]> */
    PS_PI,              /* inside <? ... ?> */
    PS_XMLDECL          /* inside <?xml ... ?> */
};

struct XML_ParserStruct {
    /* Handlers */
    XML_StartElementHandler      start_handler;
    XML_EndElementHandler        end_handler;
    XML_CharacterDataHandler     char_handler;
    XML_CommentHandler           comment_handler;
    XML_ProcessingInstructionHandler pi_handler;
    XML_StartCdataSectionHandler cdata_start_handler;
    XML_EndCdataSectionHandler   cdata_end_handler;
    XML_DefaultHandler           default_handler;
    XML_XmlDeclHandler           xmldecl_handler;

    void *user_data;

    /* Parser state */
    enum parse_state state;
    enum XML_Error   error;

    /* Buffers */
    char  name_buf[BUF_SIZE];
    int   name_len;
    char  val_buf[BUF_SIZE];
    int   val_len;
    char  text_buf[BUF_SIZE * 4];
    int   text_len;

    /* Attribute collection */
    char *attr_names[MAX_ATTR];
    char *attr_values[MAX_ATTR];
    int   attr_count;

    /* Position tracking */
    unsigned long line;
    unsigned long col;
    long          byte_index;

    /* Attribute value quote character */
    char quote_char;

    /* Comment/CDATA detection buffer */
    char  special_buf[16];
    int   special_len;

    /* Namespace separator (0 = no NS processing) */
    char ns_sep;
};

/* ========================================================================= */
/* Constructor / destructor                                                  */
/* ========================================================================= */

XML_Parser XML_ParserCreate(const XML_Char *encoding)
{
    XML_Parser p;
    (void)encoding;

    p = (XML_Parser)calloc(1, sizeof(struct XML_ParserStruct));
    if (p == NULL)
        return NULL;

    p->state = PS_OUTSIDE;
    p->error = XML_ERROR_NONE;
    p->line = 1;
    p->col = 0;
    p->byte_index = 0;
    p->ns_sep = 0;

    return p;
}

XML_Parser XML_ParserCreateNS(const XML_Char *encoding,
                               XML_Char namespaceSeparator)
{
    XML_Parser p = XML_ParserCreate(encoding);
    if (p != NULL)
        p->ns_sep = namespaceSeparator;
    return p;
}

void XML_ParserFree(XML_Parser parser)
{
    int i;
    if (parser == NULL)
        return;

    for (i = 0; i < parser->attr_count; i++) {
        free(parser->attr_names[i]);
        free(parser->attr_values[i]);
    }
    free(parser);
}

int XML_ParserReset(XML_Parser parser, const XML_Char *encoding)
{
    int i;
    (void)encoding;

    if (parser == NULL)
        return 0;

    for (i = 0; i < parser->attr_count; i++) {
        free(parser->attr_names[i]);
        free(parser->attr_values[i]);
    }

    parser->state = PS_OUTSIDE;
    parser->error = XML_ERROR_NONE;
    parser->name_len = 0;
    parser->val_len = 0;
    parser->text_len = 0;
    parser->attr_count = 0;
    parser->line = 1;
    parser->col = 0;
    parser->byte_index = 0;

    return 1;
}

/* ========================================================================= */
/* Handler setters                                                           */
/* ========================================================================= */

void XML_SetUserData(XML_Parser parser, void *userData)
{
    if (parser) parser->user_data = userData;
}

void XML_SetElementHandler(XML_Parser parser,
                           XML_StartElementHandler start,
                           XML_EndElementHandler end)
{
    if (parser) {
        parser->start_handler = start;
        parser->end_handler = end;
    }
}

void XML_SetCharacterDataHandler(XML_Parser parser,
                                 XML_CharacterDataHandler handler)
{
    if (parser) parser->char_handler = handler;
}

void XML_SetCommentHandler(XML_Parser parser,
                           XML_CommentHandler handler)
{
    if (parser) parser->comment_handler = handler;
}

void XML_SetProcessingInstructionHandler(XML_Parser parser,
    XML_ProcessingInstructionHandler handler)
{
    if (parser) parser->pi_handler = handler;
}

void XML_SetCdataSectionHandler(XML_Parser parser,
                                XML_StartCdataSectionHandler start,
                                XML_EndCdataSectionHandler end)
{
    if (parser) {
        parser->cdata_start_handler = start;
        parser->cdata_end_handler = end;
    }
}

void XML_SetDefaultHandler(XML_Parser parser,
                           XML_DefaultHandler handler)
{
    if (parser) parser->default_handler = handler;
}

void XML_SetXmlDeclHandler(XML_Parser parser,
                           XML_XmlDeclHandler handler)
{
    if (parser) parser->xmldecl_handler = handler;
}

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static void flush_text(XML_Parser p)
{
    if (p->text_len > 0 && p->char_handler) {
        p->text_buf[p->text_len] = '\0';
        p->char_handler(p->user_data, p->text_buf, p->text_len);
    }
    p->text_len = 0;
}

static void emit_start_element(XML_Parser p)
{
    const char *atts[MAX_ATTR * 2 + 1];
    int i, j = 0;

    p->name_buf[p->name_len] = '\0';

    for (i = 0; i < p->attr_count; i++) {
        atts[j++] = p->attr_names[i];
        atts[j++] = p->attr_values[i];
    }
    atts[j] = NULL;

    if (p->start_handler)
        p->start_handler(p->user_data, p->name_buf, atts);

    /* Free attribute storage */
    for (i = 0; i < p->attr_count; i++) {
        free(p->attr_names[i]);
        free(p->attr_values[i]);
    }
    p->attr_count = 0;
}

static void emit_end_element(XML_Parser p)
{
    p->name_buf[p->name_len] = '\0';
    if (p->end_handler)
        p->end_handler(p->user_data, p->name_buf);
}

static void save_attr_name(XML_Parser p)
{
    if (p->attr_count < MAX_ATTR) {
        p->val_buf[p->val_len] = '\0';
        p->attr_names[p->attr_count] = strdup(p->val_buf);
    }
    p->val_len = 0;
}

static void save_attr_value(XML_Parser p)
{
    if (p->attr_count < MAX_ATTR) {
        p->val_buf[p->val_len] = '\0';
        p->attr_values[p->attr_count] = strdup(p->val_buf);
        p->attr_count++;
    }
    p->val_len = 0;
}

static int is_name_char(char c)
{
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
           (c >= '0' && c <= '9') || c == '_' || c == '-' ||
           c == '.' || c == ':';
}

static int is_space(char c)
{
    return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

/* ========================================================================= */
/* XML_Parse -- main parsing function                                        */
/* ========================================================================= */

enum XML_Status XML_Parse(XML_Parser parser, const char *s,
                          int len, int isFinal)
{
    int i;
    char c;

    (void)isFinal;

    if (parser == NULL || s == NULL)
        return XML_STATUS_ERROR;
    if (parser->error != XML_ERROR_NONE)
        return XML_STATUS_ERROR;

    for (i = 0; i < len; i++) {
        c = s[i];
        parser->byte_index++;
        parser->col++;
        if (c == '\n') {
            parser->line++;
            parser->col = 0;
        }

        switch (parser->state) {
        case PS_OUTSIDE:
            if (c == '<') {
                flush_text(parser);
                parser->state = PS_TAG_OPEN;
                parser->name_len = 0;
                parser->special_len = 0;
            } else {
                if (parser->text_len < (int)sizeof(parser->text_buf) - 1)
                    parser->text_buf[parser->text_len++] = c;
            }
            break;

        case PS_TAG_OPEN:
            if (c == '/') {
                parser->state = PS_CLOSE_TAG_NAME;
                parser->name_len = 0;
            } else if (c == '!') {
                /* Could be comment or CDATA */
                parser->special_buf[0] = '!';
                parser->special_len = 1;
                parser->state = PS_COMMENT;
            } else if (c == '?') {
                parser->state = PS_PI;
                parser->name_len = 0;
            } else if (is_name_char(c)) {
                parser->name_buf[0] = c;
                parser->name_len = 1;
                parser->state = PS_TAG_NAME;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_TAG_NAME:
            if (is_name_char(c)) {
                if (parser->name_len < BUF_SIZE - 1)
                    parser->name_buf[parser->name_len++] = c;
            } else if (is_space(c)) {
                parser->state = PS_ATTR_SPACE;
            } else if (c == '>') {
                emit_start_element(parser);
                parser->state = PS_OUTSIDE;
            } else if (c == '/') {
                parser->state = PS_TAG_CLOSE;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_CLOSE_TAG_NAME:
            if (is_name_char(c)) {
                if (parser->name_len < BUF_SIZE - 1)
                    parser->name_buf[parser->name_len++] = c;
            } else if (c == '>') {
                emit_end_element(parser);
                parser->state = PS_OUTSIDE;
            } else if (is_space(c)) {
                /* Allow trailing whitespace before '>' */
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_ATTR_SPACE:
            if (is_space(c)) {
                /* skip whitespace */
            } else if (c == '>') {
                emit_start_element(parser);
                parser->state = PS_OUTSIDE;
            } else if (c == '/') {
                parser->state = PS_TAG_CLOSE;
            } else if (is_name_char(c)) {
                parser->val_buf[0] = c;
                parser->val_len = 1;
                parser->state = PS_ATTR_NAME;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_ATTR_NAME:
            if (is_name_char(c)) {
                if (parser->val_len < BUF_SIZE - 1)
                    parser->val_buf[parser->val_len++] = c;
            } else if (c == '=') {
                save_attr_name(parser);
                parser->state = PS_ATTR_VAL_START;
            } else if (is_space(c)) {
                save_attr_name(parser);
                parser->state = PS_ATTR_EQ;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_ATTR_EQ:
            if (is_space(c)) {
                /* skip */
            } else if (c == '=') {
                parser->state = PS_ATTR_VAL_START;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_ATTR_VAL_START:
            if (is_space(c)) {
                /* skip */
            } else if (c == '"' || c == '\'') {
                parser->quote_char = c;
                parser->val_len = 0;
                parser->state = PS_ATTR_VAL;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_ATTR_VAL:
            if (c == parser->quote_char) {
                save_attr_value(parser);
                parser->state = PS_ATTR_SPACE;
            } else {
                if (parser->val_len < BUF_SIZE - 1)
                    parser->val_buf[parser->val_len++] = c;
            }
            break;

        case PS_TAG_CLOSE:
            if (c == '>') {
                /* Self-closing tag */
                emit_start_element(parser);
                emit_end_element(parser);
                parser->state = PS_OUTSIDE;
            } else {
                parser->error = XML_ERROR_SYNTAX;
                return XML_STATUS_ERROR;
            }
            break;

        case PS_COMMENT:
            /* Detect <!-- or <![CDATA[ */
            if (parser->special_len < 8) {
                parser->special_buf[parser->special_len++] = c;

                /* Check for <!-- (comment) */
                if (parser->special_len == 3 &&
                    parser->special_buf[1] == '-' &&
                    parser->special_buf[2] == '-') {
                    /* Inside comment, wait for --> */
                    parser->text_len = 0;
                    /* Stay in PS_COMMENT, but now we're past the opening */
                }
                /* Check for <![CDATA[ */
                if (parser->special_len >= 7 &&
                    memcmp(parser->special_buf, "![CDATA[", 8) == 0) {
                    parser->state = PS_CDATA;
                    parser->text_len = 0;
                    if (parser->cdata_start_handler)
                        parser->cdata_start_handler(parser->user_data);
                }
            } else {
                /* We're inside a comment body, look for --> */
                if (parser->text_len < (int)sizeof(parser->text_buf) - 1)
                    parser->text_buf[parser->text_len++] = c;

                if (parser->text_len >= 3 &&
                    parser->text_buf[parser->text_len - 3] == '-' &&
                    parser->text_buf[parser->text_len - 2] == '-' &&
                    parser->text_buf[parser->text_len - 1] == '>') {
                    /* End of comment */
                    parser->text_len -= 3;
                    if (parser->comment_handler && parser->text_len > 0) {
                        parser->text_buf[parser->text_len] = '\0';
                        parser->comment_handler(parser->user_data,
                                                parser->text_buf);
                    }
                    parser->text_len = 0;
                    parser->state = PS_OUTSIDE;
                }
            }
            break;

        case PS_CDATA:
            if (parser->text_len < (int)sizeof(parser->text_buf) - 1)
                parser->text_buf[parser->text_len++] = c;

            if (parser->text_len >= 3 &&
                parser->text_buf[parser->text_len - 3] == ']' &&
                parser->text_buf[parser->text_len - 2] == ']' &&
                parser->text_buf[parser->text_len - 1] == '>') {
                parser->text_len -= 3;
                if (parser->char_handler && parser->text_len > 0) {
                    parser->text_buf[parser->text_len] = '\0';
                    parser->char_handler(parser->user_data,
                                         parser->text_buf,
                                         parser->text_len);
                }
                if (parser->cdata_end_handler)
                    parser->cdata_end_handler(parser->user_data);
                parser->text_len = 0;
                parser->state = PS_OUTSIDE;
            }
            break;

        case PS_PI:
            /* Processing instruction: <?target data?> */
            if (parser->text_len < (int)sizeof(parser->text_buf) - 1)
                parser->text_buf[parser->text_len++] = c;

            if (parser->text_len >= 2 &&
                parser->text_buf[parser->text_len - 2] == '?' &&
                parser->text_buf[parser->text_len - 1] == '>') {
                parser->text_len -= 2;
                parser->text_buf[parser->text_len] = '\0';
                /* Don't fire PI handler for now, just skip */
                parser->text_len = 0;
                parser->state = PS_OUTSIDE;
            }
            break;

        case PS_XMLDECL:
            /* Handled same as PI */
            if (c == '>') {
                parser->state = PS_OUTSIDE;
                parser->text_len = 0;
            }
            break;
        }
    }

    /* Flush any remaining text at the end */
    if (isFinal)
        flush_text(parser);

    return XML_STATUS_OK;
}

/* ========================================================================= */
/* Error and position queries                                                */
/* ========================================================================= */

enum XML_Error XML_GetErrorCode(XML_Parser parser)
{
    if (parser == NULL)
        return XML_ERROR_NO_MEMORY;
    return parser->error;
}

static const char *error_strings[] = {
    "no error",
    "out of memory",
    "syntax error",
    "no element found",
    "invalid token",
    "unclosed token",
    "partial character",
    "tag mismatch",
    "duplicate attribute",
    "junk after document element",
    "parameter entity reference",
    "undefined entity",
    "recursive entity reference",
    "async entity",
    "bad character reference",
    "binary entity reference",
    "attribute external entity reference",
    "misplaced XML processing instruction",
    "unknown encoding",
    "incorrect encoding",
    "unclosed CDATA section",
    "external entity handling",
    "not standalone",
    "unexpected state",
    "aborted",
    "finished",
    "suspend PE"
};

const XML_LChar *XML_ErrorString(enum XML_Error code)
{
    if (code >= 0 && code <= XML_ERROR_SUSPEND_PE)
        return error_strings[code];
    return "unknown error";
}

XML_Size XML_GetCurrentLineNumber(XML_Parser parser)
{
    return parser ? parser->line : 0;
}

XML_Size XML_GetCurrentColumnNumber(XML_Parser parser)
{
    return parser ? parser->col : 0;
}

XML_Index XML_GetCurrentByteIndex(XML_Parser parser)
{
    return parser ? parser->byte_index : -1;
}

const XML_LChar *XML_ExpatVersion(void)
{
    return "expat_2.6.0";
}
