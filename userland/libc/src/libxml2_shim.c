/*
 * VeridianOS libc -- libxml2_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libxml2 2.13.x shim.
 * Uses the expat shim as a SAX backend, building a DOM tree on top.
 * Provides xmlParseMemory, xmlParseFile, DOM traversal, XPath stubs.
 */

#include <libxml/parser.h>
#include <libxml/tree.h>
#include <libxml/xpath.h>
#include <libxml/xmlstring.h>
#include <libxml/xmlmemory.h>
#include <expat.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Memory management                                                         */
/* ========================================================================= */

void xmlFree(void *mem)
{
    free(mem);
}

void *xmlMalloc(size_t size)
{
    return malloc(size);
}

void *xmlRealloc(void *mem, size_t size)
{
    return realloc(mem, size);
}

char *xmlMemStrdup(const char *str)
{
    return strdup(str);
}

int xmlMemSetup(void (*freeFunc)(void *),
                void *(*mallocFunc)(size_t),
                void *(*reallocFunc)(void *, size_t),
                char *(*strdupFunc)(const char *))
{
    (void)freeFunc;
    (void)mallocFunc;
    (void)reallocFunc;
    (void)strdupFunc;
    return 0;
}

void xmlMemoryDump(void) { }

/* ========================================================================= */
/* xmlString operations                                                      */
/* ========================================================================= */

int xmlStrlen(const xmlChar *str)
{
    if (str == NULL) return 0;
    return (int)strlen((const char *)str);
}

int xmlStrcmp(const xmlChar *str1, const xmlChar *str2)
{
    if (str1 == NULL && str2 == NULL) return 0;
    if (str1 == NULL) return -1;
    if (str2 == NULL) return 1;
    return strcmp((const char *)str1, (const char *)str2);
}

int xmlStrcasecmp(const xmlChar *str1, const xmlChar *str2)
{
    if (str1 == NULL && str2 == NULL) return 0;
    if (str1 == NULL) return -1;
    if (str2 == NULL) return 1;

    while (*str1 && *str2) {
        unsigned char c1 = *str1, c2 = *str2;
        if (c1 >= 'A' && c1 <= 'Z') c1 += 32;
        if (c2 >= 'A' && c2 <= 'Z') c2 += 32;
        if (c1 != c2) return (int)c1 - (int)c2;
        str1++;
        str2++;
    }
    return (int)*str1 - (int)*str2;
}

int xmlStrncmp(const xmlChar *str1, const xmlChar *str2, int len)
{
    return strncmp((const char *)str1, (const char *)str2, (size_t)len);
}

xmlChar *xmlStrdup(const xmlChar *cur)
{
    if (cur == NULL) return NULL;
    return (xmlChar *)strdup((const char *)cur);
}

xmlChar *xmlStrndup(const xmlChar *cur, int len)
{
    xmlChar *ret;
    if (cur == NULL || len < 0) return NULL;
    ret = (xmlChar *)malloc((size_t)len + 1);
    if (ret == NULL) return NULL;
    memcpy(ret, cur, (size_t)len);
    ret[len] = '\0';
    return ret;
}

xmlChar *xmlStrcat(xmlChar *cur, const xmlChar *add)
{
    xmlChar *ret;
    int len1, len2;

    if (add == NULL) return cur;
    if (cur == NULL) return xmlStrdup(add);

    len1 = xmlStrlen(cur);
    len2 = xmlStrlen(add);
    ret = (xmlChar *)realloc(cur, (size_t)(len1 + len2 + 1));
    if (ret == NULL) return cur;
    memcpy(ret + len1, add, (size_t)len2 + 1);
    return ret;
}

const xmlChar *xmlStrstr(const xmlChar *str, const xmlChar *val)
{
    if (str == NULL || val == NULL) return NULL;
    return (const xmlChar *)strstr((const char *)str, (const char *)val);
}

const xmlChar *xmlStrchr(const xmlChar *str, xmlChar val)
{
    if (str == NULL) return NULL;
    return (const xmlChar *)strchr((const char *)str, (char)val);
}

int xmlStrPrintf(xmlChar *buf, int len, const char *msg, ...)
{
    (void)buf;
    (void)len;
    (void)msg;
    return 0;  /* Stub */
}

/* ========================================================================= */
/* DOM node construction                                                     */
/* ========================================================================= */

xmlDocPtr xmlNewDoc(const xmlChar *version)
{
    xmlDocPtr doc = (xmlDocPtr)calloc(1, sizeof(xmlDoc));
    if (doc == NULL) return NULL;
    doc->type = XML_DOCUMENT_NODE;
    doc->version = xmlStrdup(version ? version : (const xmlChar *)"1.0");
    doc->doc = doc;
    return doc;
}

static void free_node_tree(xmlNodePtr node);

static void free_attr_list(xmlAttrPtr attr)
{
    while (attr) {
        xmlAttrPtr next = attr->next;
        xmlFree((void *)attr->name);
        if (attr->children)
            free_node_tree(attr->children);
        free(attr);
        attr = next;
    }
}

static void free_node_tree(xmlNodePtr node)
{
    while (node) {
        xmlNodePtr next = node->next;
        xmlFree((void *)node->name);
        xmlFree(node->content);
        free_attr_list(node->properties);
        if (node->children)
            free_node_tree(node->children);
        free(node);
        node = next;
    }
}

void xmlFreeDoc(xmlDocPtr cur)
{
    if (cur == NULL) return;
    xmlFree((void *)cur->version);
    xmlFree((void *)cur->encoding);
    xmlFree((void *)cur->URL);
    if (cur->children)
        free_node_tree(cur->children);
    free(cur);
}

xmlNodePtr xmlDocGetRootElement(const xmlDoc *doc)
{
    xmlNodePtr cur;
    if (doc == NULL) return NULL;
    cur = doc->children;
    while (cur) {
        if (cur->type == XML_ELEMENT_NODE)
            return cur;
        cur = cur->next;
    }
    return NULL;
}

xmlNodePtr xmlDocSetRootElement(xmlDocPtr doc, xmlNodePtr root)
{
    xmlNodePtr old;
    if (doc == NULL) return NULL;
    old = xmlDocGetRootElement(doc);
    if (old)
        old->parent = NULL;
    doc->children = root;
    if (root) {
        root->parent = (xmlNodePtr)doc;
        root->doc = doc;
    }
    return old;
}

xmlNodePtr xmlNewNode(xmlNsPtr ns, const xmlChar *name)
{
    xmlNodePtr node = (xmlNodePtr)calloc(1, sizeof(xmlNode));
    if (node == NULL) return NULL;
    node->type = XML_ELEMENT_NODE;
    node->name = xmlStrdup(name);
    node->ns = ns;
    return node;
}

xmlNodePtr xmlNewText(const xmlChar *content)
{
    xmlNodePtr node = (xmlNodePtr)calloc(1, sizeof(xmlNode));
    if (node == NULL) return NULL;
    node->type = XML_TEXT_NODE;
    node->name = xmlStrdup((const xmlChar *)"text");
    node->content = xmlStrdup(content);
    return node;
}

xmlNodePtr xmlAddChild(xmlNodePtr parent, xmlNodePtr cur)
{
    if (parent == NULL || cur == NULL) return NULL;
    cur->parent = parent;
    cur->doc = parent->doc;
    if (parent->last == NULL) {
        parent->children = cur;
        parent->last = cur;
    } else {
        parent->last->next = cur;
        cur->prev = parent->last;
        parent->last = cur;
    }
    return cur;
}

xmlNodePtr xmlAddNextSibling(xmlNodePtr cur, xmlNodePtr elem)
{
    if (cur == NULL || elem == NULL) return NULL;
    elem->parent = cur->parent;
    elem->doc = cur->doc;
    elem->prev = cur;
    elem->next = cur->next;
    cur->next = elem;
    if (elem->next)
        elem->next->prev = elem;
    else if (elem->parent)
        elem->parent->last = elem;
    return elem;
}

xmlNodePtr xmlAddPrevSibling(xmlNodePtr cur, xmlNodePtr elem)
{
    if (cur == NULL || elem == NULL) return NULL;
    elem->parent = cur->parent;
    elem->doc = cur->doc;
    elem->next = cur;
    elem->prev = cur->prev;
    cur->prev = elem;
    if (elem->prev)
        elem->prev->next = elem;
    else if (elem->parent)
        elem->parent->children = elem;
    return elem;
}

void xmlUnlinkNode(xmlNodePtr cur)
{
    if (cur == NULL) return;
    if (cur->prev)
        cur->prev->next = cur->next;
    else if (cur->parent)
        cur->parent->children = cur->next;
    if (cur->next)
        cur->next->prev = cur->prev;
    else if (cur->parent)
        cur->parent->last = cur->prev;
    cur->parent = NULL;
    cur->prev = NULL;
    cur->next = NULL;
}

void xmlFreeNode(xmlNodePtr cur)
{
    if (cur == NULL) return;
    xmlUnlinkNode(cur);
    xmlFree((void *)cur->name);
    xmlFree(cur->content);
    free_attr_list(cur->properties);
    if (cur->children)
        free_node_tree(cur->children);
    free(cur);
}

xmlChar *xmlNodeGetContent(const xmlNode *cur)
{
    if (cur == NULL) return NULL;
    if (cur->content)
        return xmlStrdup(cur->content);
    /* Concatenate text children */
    if (cur->children && cur->children->type == XML_TEXT_NODE)
        return xmlStrdup(cur->children->content);
    return xmlStrdup((const xmlChar *)"");
}

void xmlNodeSetContent(xmlNodePtr cur, const xmlChar *content)
{
    if (cur == NULL) return;
    xmlFree(cur->content);
    cur->content = xmlStrdup(content);
}

xmlChar *xmlGetProp(const xmlNode *node, const xmlChar *name)
{
    xmlAttrPtr attr;
    if (node == NULL || name == NULL) return NULL;
    attr = node->properties;
    while (attr) {
        if (xmlStrcmp(attr->name, name) == 0) {
            if (attr->children && attr->children->content)
                return xmlStrdup(attr->children->content);
            return xmlStrdup((const xmlChar *)"");
        }
        attr = attr->next;
    }
    return NULL;
}

xmlAttrPtr xmlSetProp(xmlNodePtr node, const xmlChar *name,
                      const xmlChar *value)
{
    xmlAttrPtr attr;
    xmlNodePtr text;

    if (node == NULL || name == NULL) return NULL;

    /* Check if attribute already exists */
    attr = node->properties;
    while (attr) {
        if (xmlStrcmp(attr->name, name) == 0) {
            if (attr->children) {
                xmlFree(attr->children->content);
                attr->children->content = xmlStrdup(value);
            }
            return attr;
        }
        attr = attr->next;
    }

    /* Create new attribute */
    attr = (xmlAttrPtr)calloc(1, sizeof(xmlAttr));
    if (attr == NULL) return NULL;
    attr->type = XML_ATTRIBUTE_NODE;
    attr->name = xmlStrdup(name);
    attr->parent = node;
    attr->doc = node->doc;

    text = xmlNewText(value);
    if (text) {
        attr->children = text;
        text->parent = (xmlNodePtr)attr;
    }

    /* Prepend to properties list */
    attr->next = node->properties;
    node->properties = attr;

    return attr;
}

xmlAttrPtr xmlHasProp(const xmlNode *node, const xmlChar *name)
{
    xmlAttrPtr attr;
    if (node == NULL || name == NULL) return NULL;
    attr = node->properties;
    while (attr) {
        if (xmlStrcmp(attr->name, name) == 0)
            return attr;
        attr = attr->next;
    }
    return NULL;
}

int xmlRemoveProp(xmlAttrPtr attr)
{
    (void)attr;
    return 0;  /* Stub */
}

xmlNsPtr xmlNewNs(xmlNodePtr node, const xmlChar *href,
                  const xmlChar *prefix)
{
    (void)node; (void)href; (void)prefix;
    return NULL;  /* Stub */
}

void xmlSetNs(xmlNodePtr node, xmlNsPtr ns)
{
    if (node) node->ns = ns;
}

/* ========================================================================= */
/* SAX-to-DOM builder using expat                                            */
/* ========================================================================= */

struct parse_ctx {
    xmlDocPtr  doc;
    xmlNodePtr current;
    int        error;
};

static void sax_start(void *userData, const XML_Char *name,
                       const XML_Char **atts)
{
    struct parse_ctx *ctx = (struct parse_ctx *)userData;
    xmlNodePtr node;
    int i;

    node = xmlNewNode(NULL, (const xmlChar *)name);
    if (node == NULL) {
        ctx->error = 1;
        return;
    }
    node->doc = ctx->doc;

    /* Add attributes */
    for (i = 0; atts[i] != NULL; i += 2) {
        xmlSetProp(node, (const xmlChar *)atts[i],
                   (const xmlChar *)atts[i + 1]);
    }

    if (ctx->current == NULL) {
        xmlDocSetRootElement(ctx->doc, node);
    } else {
        xmlAddChild(ctx->current, node);
    }
    ctx->current = node;
}

static void sax_end(void *userData, const XML_Char *name)
{
    struct parse_ctx *ctx = (struct parse_ctx *)userData;
    (void)name;
    if (ctx->current && ctx->current->parent) {
        /* Move up to parent, unless parent is the document */
        if (ctx->current->parent->type == XML_ELEMENT_NODE)
            ctx->current = ctx->current->parent;
        else
            ctx->current = NULL;
    }
}

static void sax_chardata(void *userData, const XML_Char *s, int len)
{
    struct parse_ctx *ctx = (struct parse_ctx *)userData;
    xmlNodePtr text;
    xmlChar *content;

    if (ctx->current == NULL || len <= 0)
        return;

    content = xmlStrndup((const xmlChar *)s, len);
    if (content == NULL) return;

    text = xmlNewText(content);
    xmlFree(content);
    if (text == NULL) return;

    text->doc = ctx->doc;
    xmlAddChild(ctx->current, text);
}

/* ========================================================================= */
/* Parser API                                                                */
/* ========================================================================= */

static xmlError last_xml_error;

void xmlInitParser(void) { }
void xmlCleanupParser(void) { }

xmlDocPtr xmlParseMemory(const char *buffer, int size)
{
    return xmlReadMemory(buffer, size, NULL, NULL, 0);
}

xmlDocPtr xmlReadMemory(const char *buffer, int size,
                        const char *URL, const char *encoding,
                        int options)
{
    struct parse_ctx ctx;
    XML_Parser parser;
    enum XML_Status status;

    (void)URL;
    (void)encoding;
    (void)options;

    if (buffer == NULL || size <= 0)
        return NULL;

    ctx.doc = xmlNewDoc((const xmlChar *)"1.0");
    if (ctx.doc == NULL)
        return NULL;

    ctx.current = NULL;
    ctx.error = 0;

    parser = XML_ParserCreate(NULL);
    if (parser == NULL) {
        xmlFreeDoc(ctx.doc);
        return NULL;
    }

    XML_SetUserData(parser, &ctx);
    XML_SetElementHandler(parser, sax_start, sax_end);
    XML_SetCharacterDataHandler(parser, sax_chardata);

    status = XML_Parse(parser, buffer, size, 1);
    XML_ParserFree(parser);

    if (status != XML_STATUS_OK || ctx.error) {
        xmlFreeDoc(ctx.doc);
        return NULL;
    }

    return ctx.doc;
}

xmlDocPtr xmlParseFile(const char *filename)
{
    (void)filename;
    return NULL;  /* File I/O requires filesystem integration */
}

xmlDocPtr xmlReadFile(const char *filename, const char *encoding,
                      int options)
{
    (void)filename;
    (void)encoding;
    (void)options;
    return NULL;
}

xmlErrorPtr xmlGetLastError(void)
{
    return &last_xml_error;
}

void xmlResetLastError(void)
{
    memset(&last_xml_error, 0, sizeof(last_xml_error));
}

int xmlDocDump(void *f, xmlDocPtr cur)
{
    (void)f;
    (void)cur;
    return -1;
}

void xmlDocDumpMemory(xmlDocPtr cur, xmlChar **mem, int *size)
{
    (void)cur;
    if (mem) *mem = NULL;
    if (size) *size = 0;
}

void xmlDocDumpFormatMemory(xmlDocPtr cur, xmlChar **mem, int *size,
                            int format)
{
    (void)format;
    xmlDocDumpMemory(cur, mem, size);
}

/* ========================================================================= */
/* XPath stubs                                                               */
/* ========================================================================= */

xmlXPathContextPtr xmlXPathNewContext(xmlDocPtr doc)
{
    xmlXPathContextPtr ctx;
    ctx = (xmlXPathContextPtr)calloc(1, sizeof(xmlXPathContext));
    if (ctx) {
        ctx->doc = doc;
        ctx->node = xmlDocGetRootElement(doc);
    }
    return ctx;
}

void xmlXPathFreeContext(xmlXPathContextPtr ctxt)
{
    free(ctxt);
}

xmlXPathObjectPtr xmlXPathEval(const xmlChar *str,
                               xmlXPathContextPtr ctx)
{
    (void)str;
    (void)ctx;
    return NULL;  /* XPath evaluation not implemented */
}

xmlXPathObjectPtr xmlXPathEvalExpression(const xmlChar *str,
                                         xmlXPathContextPtr ctxt)
{
    return xmlXPathEval(str, ctxt);
}

void xmlXPathFreeObject(xmlXPathObjectPtr obj)
{
    if (obj == NULL) return;
    if (obj->stringval)
        xmlFree(obj->stringval);
    if (obj->nodesetval) {
        free(obj->nodesetval->nodeTab);
        free(obj->nodesetval);
    }
    free(obj);
}

int xmlXPathRegisterNs(xmlXPathContextPtr ctxt,
                       const xmlChar *prefix, const xmlChar *ns_uri)
{
    (void)ctxt;
    (void)prefix;
    (void)ns_uri;
    return 0;
}

void xmlXPathInit(void) { }
