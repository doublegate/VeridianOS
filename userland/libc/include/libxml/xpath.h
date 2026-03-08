/*
 * VeridianOS libc -- libxml/xpath.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 XPath evaluation API.
 */

#ifndef _LIBXML_XPATH_H
#define _LIBXML_XPATH_H

#include "tree.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* XPath object types                                                        */
/* ========================================================================= */

typedef enum {
    XPATH_UNDEFINED   = 0,
    XPATH_NODESET     = 1,
    XPATH_BOOLEAN     = 2,
    XPATH_NUMBER      = 3,
    XPATH_STRING      = 4,
    XPATH_USERS       = 8,
    XPATH_XSLT_TREE   = 9
} xmlXPathObjectType;

/* ========================================================================= */
/* XPath structures                                                          */
/* ========================================================================= */

typedef struct _xmlXPathContext  xmlXPathContext;
typedef xmlXPathContext *xmlXPathContextPtr;

typedef struct _xmlNodeSet {
    int nodeNr;
    int nodeMax;
    xmlNodePtr *nodeTab;
} xmlNodeSet;

typedef xmlNodeSet *xmlNodeSetPtr;

typedef struct _xmlXPathObject {
    xmlXPathObjectType type;
    xmlNodeSetPtr nodesetval;
    int boolval;
    double floatval;
    xmlChar *stringval;
    void *user;
    int index;
    void *user2;
    int index2;
} xmlXPathObject;

typedef xmlXPathObject *xmlXPathObjectPtr;

struct _xmlXPathContext {
    xmlDocPtr doc;
    xmlNodePtr node;
    /* Internal fields omitted */
    void *_private;
};

/* ========================================================================= */
/* XPath API                                                                 */
/* ========================================================================= */

/** Create a new XPath context. */
xmlXPathContextPtr xmlXPathNewContext(xmlDocPtr doc);

/** Free an XPath context. */
void xmlXPathFreeContext(xmlXPathContextPtr ctxt);

/** Evaluate an XPath expression. */
xmlXPathObjectPtr xmlXPathEval(const xmlChar *str,
                               xmlXPathContextPtr ctx);

/** Evaluate a compiled XPath expression. */
xmlXPathObjectPtr xmlXPathEvalExpression(const xmlChar *str,
                                         xmlXPathContextPtr ctxt);

/** Free an XPath object. */
void xmlXPathFreeObject(xmlXPathObjectPtr obj);

/** Register a namespace for XPath. */
int xmlXPathRegisterNs(xmlXPathContextPtr ctxt,
                       const xmlChar *prefix, const xmlChar *ns_uri);

/** Initialize XPath. */
void xmlXPathInit(void);

#ifdef __cplusplus
}
#endif

#endif /* _LIBXML_XPATH_H */
