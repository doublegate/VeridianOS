/*
 * VeridianOS libc -- libxml/tree.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 DOM tree structures and API.
 */

#ifndef _LIBXML_TREE_H
#define _LIBXML_TREE_H

#include "xmlstring.h"
#include "xmlversion.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Node types                                                                */
/* ========================================================================= */

typedef enum {
    XML_ELEMENT_NODE       = 1,
    XML_ATTRIBUTE_NODE     = 2,
    XML_TEXT_NODE          = 3,
    XML_CDATA_SECTION_NODE = 4,
    XML_ENTITY_REF_NODE    = 5,
    XML_ENTITY_NODE        = 6,
    XML_PI_NODE            = 7,
    XML_COMMENT_NODE       = 8,
    XML_DOCUMENT_NODE      = 9,
    XML_DOCUMENT_TYPE_NODE = 10,
    XML_DOCUMENT_FRAG_NODE = 11,
    XML_NOTATION_NODE      = 12,
    XML_HTML_DOCUMENT_NODE = 13,
    XML_DTD_NODE           = 14,
    XML_ELEMENT_DECL       = 15,
    XML_ATTRIBUTE_DECL     = 16,
    XML_ENTITY_DECL        = 17,
    XML_NAMESPACE_DECL     = 18,
    XML_XINCLUDE_START     = 19,
    XML_XINCLUDE_END       = 20
} xmlElementType;

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

typedef struct _xmlDoc   xmlDoc;
typedef struct _xmlNode  xmlNode;
typedef struct _xmlAttr  xmlAttr;
typedef struct _xmlNs    xmlNs;
typedef struct _xmlDtd   xmlDtd;

typedef xmlDoc  *xmlDocPtr;
typedef xmlNode *xmlNodePtr;
typedef xmlAttr *xmlAttrPtr;
typedef xmlNs   *xmlNsPtr;

/* ========================================================================= */
/* Structures                                                                */
/* ========================================================================= */

struct _xmlNs {
    struct _xmlNs *next;
    xmlElementType type;
    const xmlChar *href;
    const xmlChar *prefix;
    void *_private;
    struct _xmlDoc *context;
};

struct _xmlAttr {
    void *_private;
    xmlElementType type;
    const xmlChar *name;
    struct _xmlNode *children;
    struct _xmlNode *last;
    struct _xmlNode *parent;
    struct _xmlAttr *next;
    struct _xmlAttr *prev;
    struct _xmlDoc *doc;
    xmlNs *ns;
    xmlChar *content;  /* attribute value as text */
};

struct _xmlNode {
    void *_private;
    xmlElementType type;
    const xmlChar *name;
    struct _xmlNode *children;
    struct _xmlNode *last;
    struct _xmlNode *parent;
    struct _xmlNode *next;
    struct _xmlNode *prev;
    struct _xmlDoc *doc;
    xmlNs *ns;
    xmlChar *content;
    struct _xmlAttr *properties;
    xmlNs *nsDef;
    void *psvi;
    unsigned short line;
    unsigned short extra;
};

struct _xmlDoc {
    void *_private;
    xmlElementType type;
    char *name;
    struct _xmlNode *children;
    struct _xmlNode *last;
    struct _xmlNode *parent;
    struct _xmlNode *next;
    struct _xmlNode *prev;
    struct _xmlDoc *doc;
    int compression;
    int standalone;
    struct _xmlDtd *intSubset;
    struct _xmlDtd *extSubset;
    struct _xmlNs *oldNs;
    const xmlChar *version;
    const xmlChar *encoding;
    void *ids;
    void *refs;
    const xmlChar *URL;
    int charset;
    struct _xmlDtd *dict;
    void *psvi;
    int parseFlags;
    int properties;
};

/* ========================================================================= */
/* Document API                                                              */
/* ========================================================================= */

/** Create a new document. */
xmlDocPtr xmlNewDoc(const xmlChar *version);

/** Free a document tree. */
void xmlFreeDoc(xmlDocPtr cur);

/** Get the root element of a document. */
xmlNodePtr xmlDocGetRootElement(const xmlDoc *doc);

/** Set the root element of a document. */
xmlNodePtr xmlDocSetRootElement(xmlDocPtr doc, xmlNodePtr root);

/* ========================================================================= */
/* Node API                                                                  */
/* ========================================================================= */

/** Create a new node. */
xmlNodePtr xmlNewNode(xmlNsPtr ns, const xmlChar *name);

/** Create a new text node. */
xmlNodePtr xmlNewText(const xmlChar *content);

/** Add a child to a node. */
xmlNodePtr xmlAddChild(xmlNodePtr parent, xmlNodePtr cur);

/** Add a sibling after a node. */
xmlNodePtr xmlAddNextSibling(xmlNodePtr cur, xmlNodePtr elem);

/** Add a sibling before a node. */
xmlNodePtr xmlAddPrevSibling(xmlNodePtr cur, xmlNodePtr elem);

/** Unlink a node from its tree. */
void xmlUnlinkNode(xmlNodePtr cur);

/** Free a node and all its children. */
void xmlFreeNode(xmlNodePtr cur);

/** Get the text content of a node. */
xmlChar *xmlNodeGetContent(const xmlNode *cur);

/** Set the text content of a node. */
void xmlNodeSetContent(xmlNodePtr cur, const xmlChar *content);

/** Get a property (attribute) value by name. */
xmlChar *xmlGetProp(const xmlNode *node, const xmlChar *name);

/** Set a property (attribute) on a node. */
xmlAttrPtr xmlSetProp(xmlNodePtr node, const xmlChar *name,
                      const xmlChar *value);

/** Check if a node has a property. */
xmlAttrPtr xmlHasProp(const xmlNode *node, const xmlChar *name);

/** Remove a property. */
int xmlRemoveProp(xmlAttrPtr attr);

/** Create a new namespace. */
xmlNsPtr xmlNewNs(xmlNodePtr node, const xmlChar *href,
                  const xmlChar *prefix);

/** Set the namespace on a node. */
void xmlSetNs(xmlNodePtr node, xmlNsPtr ns);

#ifdef __cplusplus
}
#endif

#endif /* _LIBXML_TREE_H */
