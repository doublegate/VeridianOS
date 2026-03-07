/*
 * VeridianOS libc -- cxa_exception.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * C++ exception handling runtime.
 *
 * This implements the Itanium C++ ABI exception handling functions:
 *   - __cxa_allocate_exception / __cxa_free_exception
 *   - __cxa_throw / __cxa_rethrow
 *   - __cxa_begin_catch / __cxa_end_catch
 *   - __cxa_get_exception_ptr / __cxa_current_exception_type
 *   - __cxa_get_globals / __cxa_get_globals_fast
 *   - __gxx_personality_v0 (C++ personality routine)
 *
 * The personality routine decodes the LSDA (Language-Specific Data Area)
 * to find catch handlers and cleanup actions.  It works together with
 * the unwinder (unwind.c) to implement two-phase exception handling.
 *
 * Thread-safety:
 *   Per-thread exception state is stored in a static __cxa_eh_globals
 *   struct.  When full TLS support is available, this should use
 *   __thread storage.  For now, a single global suffices for the
 *   initial single-threaded userland.
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi-eh.html
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <unwind.h>

/* Forward-declare to avoid pulling in full headers */
void *malloc(size_t size);
void free(void *ptr);
long write(int fd, const void *buf, unsigned long count);
void abort(void) __attribute__((noreturn));

/* ========================================================================= */
/* Exception class identifier                                                */
/* ========================================================================= */

/*
 * The Itanium ABI uses an 8-byte exception class to identify the
 * language/vendor.  "GNUCC++\0" is the standard for GCC C++ exceptions.
 */
static const uint64_t CXX_EXCEPTION_CLASS = 0x474E5543432B2B00ULL; /* GNUCC++ */

/* ========================================================================= */
/* Internal exception header                                                 */
/* ========================================================================= */

/*
 * __cxa_exception is prepended to every thrown exception object.
 * The user-visible exception object pointer points just past this header.
 *
 * Layout:
 *   [__cxa_exception header][user exception object]
 *                           ^--- pointer returned to user
 */
struct __cxa_exception {
    /* Type info for this exception (std::type_info pointer) */
    void *exceptionType;

    /* Destructor for the exception object, or NULL */
    void (*exceptionDestructor)(void *);

    /* Handler that caught this exception (for rethrow) */
    void (*unexpectedHandler)(void);
    void (*terminateHandler)(void);

    /* Linked list of caught exceptions (per-thread) */
    struct __cxa_exception *nextException;

    /* Number of active catch handlers for this exception */
    int handlerCount;

    /* Number of active handler switches (rethrow tracking) */
    int handlerSwitchValue;

    /* Action record for personality routine */
    const char *actionRecord;

    /* LSDA pointer cached from personality */
    const char *languageSpecificData;

    /* Landing pad / catch handler pointer */
    void *catchTemp;

    /* Adjusted pointer for the caught exception */
    void *adjustedPtr;

    /* The Itanium ABI unwind header -- MUST be last so that
     * (struct _Unwind_Exception *) aligns with what the unwinder sees */
    struct _Unwind_Exception unwindHeader;
};

/* ========================================================================= */
/* Per-thread exception state                                                */
/* ========================================================================= */

struct __cxa_eh_globals {
    struct __cxa_exception *caughtExceptions;
    unsigned int uncaughtExceptions;
};

/*
 * Single-threaded globals.  When VeridianOS gains per-thread TLS,
 * this should become __thread-qualified.
 */
static struct __cxa_eh_globals eh_globals;

struct __cxa_eh_globals *__cxa_get_globals(void)
{
    return &eh_globals;
}

struct __cxa_eh_globals *__cxa_get_globals_fast(void)
{
    return &eh_globals;
}

/* ========================================================================= */
/* Helper: convert between user pointer and __cxa_exception header           */
/* ========================================================================= */

static struct __cxa_exception *to_cxa_exception(void *thrown_object)
{
    return ((struct __cxa_exception *)thrown_object) - 1;
}

/* ========================================================================= */
/* __cxa_allocate_exception                                                  */
/* ========================================================================= */

/*
 * Allocate space for __cxa_exception header + user object.
 * The returned pointer points to the user object (past the header).
 */
void *__cxa_allocate_exception(size_t thrown_size)
{
    size_t total = sizeof(struct __cxa_exception) + thrown_size;
    void *p = malloc(total);

    if (!p) {
        /* Exception allocation failure is fatal per the ABI */
        static const char msg[] = "__cxa_allocate_exception: out of memory\n";
        write(2, msg, sizeof(msg) - 1);
        abort();
    }

    memset(p, 0, total);

    /* Return pointer past the header */
    return (char *)p + sizeof(struct __cxa_exception);
}

/* ========================================================================= */
/* __cxa_free_exception                                                      */
/* ========================================================================= */

void __cxa_free_exception(void *thrown_exception)
{
    if (!thrown_exception)
        return;

    struct __cxa_exception *header = to_cxa_exception(thrown_exception);
    free(header);
}

/* ========================================================================= */
/* Exception cleanup callback (called by _Unwind_DeleteException)            */
/* ========================================================================= */

static void exception_cleanup(_Unwind_Reason_Code reason,
                              struct _Unwind_Exception *exc)
{
    (void)reason;

    /* Get the __cxa_exception containing this unwind header */
    struct __cxa_exception *header =
        (struct __cxa_exception *)((char *)exc -
            __builtin_offsetof(struct __cxa_exception, unwindHeader));

    void *thrown = header + 1; /* user object pointer */

    if (header->exceptionDestructor)
        header->exceptionDestructor(thrown);

    free(header);
}

/* ========================================================================= */
/* __cxa_throw                                                               */
/* ========================================================================= */

void __cxa_throw(void *thrown_exception, void *tinfo, void (*dest)(void *))
{
    struct __cxa_exception *header = to_cxa_exception(thrown_exception);

    header->exceptionType       = tinfo;
    header->exceptionDestructor = dest;
    header->unexpectedHandler   = (void *)0;
    header->terminateHandler    = (void *)0;

    header->unwindHeader.exception_class   = CXX_EXCEPTION_CLASS;
    header->unwindHeader.exception_cleanup = exception_cleanup;

    struct __cxa_eh_globals *globals = __cxa_get_globals();
    globals->uncaughtExceptions++;

    _Unwind_Reason_Code rc = _Unwind_RaiseException(&header->unwindHeader);

    /*
     * If _Unwind_RaiseException returns, no handler was found.
     * The standard says to call std::terminate().
     */
    (void)rc;
    static const char msg[] = "__cxa_throw: no handler found, calling abort\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/* ========================================================================= */
/* __cxa_begin_catch                                                         */
/* ========================================================================= */

void *__cxa_begin_catch(void *exception_header)
{
    struct _Unwind_Exception *unwind_exc =
        (struct _Unwind_Exception *)exception_header;

    struct __cxa_exception *header =
        (struct __cxa_exception *)((char *)unwind_exc -
            __builtin_offsetof(struct __cxa_exception, unwindHeader));

    struct __cxa_eh_globals *globals = __cxa_get_globals();

    /* Push onto caught exceptions list */
    header->nextException = globals->caughtExceptions;
    globals->caughtExceptions = header;
    header->handlerCount++;

    globals->uncaughtExceptions--;

    /* Return pointer to the user exception object */
    return header + 1;
}

/* ========================================================================= */
/* __cxa_end_catch                                                           */
/* ========================================================================= */

void __cxa_end_catch(void)
{
    struct __cxa_eh_globals *globals = __cxa_get_globals_fast();

    if (!globals->caughtExceptions)
        return;

    struct __cxa_exception *header = globals->caughtExceptions;
    header->handlerCount--;

    if (header->handlerCount == 0) {
        /* Remove from caught list */
        globals->caughtExceptions = header->nextException;

        /* Destroy and free */
        void *thrown = header + 1;
        if (header->exceptionDestructor)
            header->exceptionDestructor(thrown);
        free(header);
    }
}

/* ========================================================================= */
/* __cxa_rethrow                                                             */
/* ========================================================================= */

void __cxa_rethrow(void)
{
    struct __cxa_eh_globals *globals = __cxa_get_globals();

    if (!globals->caughtExceptions) {
        static const char msg[] = "__cxa_rethrow: no current exception\n";
        write(2, msg, sizeof(msg) - 1);
        abort();
    }

    struct __cxa_exception *header = globals->caughtExceptions;
    globals->uncaughtExceptions++;

    /* Remove from caught list and re-raise */
    globals->caughtExceptions = header->nextException;
    header->nextException = (void *)0;

    _Unwind_Reason_Code rc = _Unwind_RaiseException(&header->unwindHeader);
    (void)rc;

    static const char msg[] = "__cxa_rethrow: no handler found\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/* ========================================================================= */
/* __cxa_get_exception_ptr                                                   */
/* ========================================================================= */

void *__cxa_get_exception_ptr(void *exception_header)
{
    struct _Unwind_Exception *unwind_exc =
        (struct _Unwind_Exception *)exception_header;

    struct __cxa_exception *header =
        (struct __cxa_exception *)((char *)unwind_exc -
            __builtin_offsetof(struct __cxa_exception, unwindHeader));

    return header + 1;
}

/* ========================================================================= */
/* __cxa_current_exception_type                                              */
/* ========================================================================= */

void *__cxa_current_exception_type(void)
{
    struct __cxa_eh_globals *globals = __cxa_get_globals_fast();

    if (!globals->caughtExceptions)
        return (void *)0;

    return globals->caughtExceptions->exceptionType;
}

/* ========================================================================= */
/* LSDA (Language-Specific Data Area) parsing helpers                        */
/* ========================================================================= */

/*
 * The LSDA is generated by the compiler for each function that has
 * try/catch blocks or objects needing cleanup.  It contains:
 *   - Call site table: maps IP ranges to landing pads and action records
 *   - Action table: lists type filters for catch clauses
 *   - Type table: pointers to std::type_info objects
 *
 * DWARF encoding types used in the LSDA:
 */
#define DW_EH_PE_absptr   0x00
#define DW_EH_PE_uleb128  0x01
#define DW_EH_PE_udata2   0x02
#define DW_EH_PE_udata4   0x03
#define DW_EH_PE_udata8   0x04
#define DW_EH_PE_sleb128  0x09
#define DW_EH_PE_sdata2   0x0A
#define DW_EH_PE_sdata4   0x0B
#define DW_EH_PE_sdata8   0x0C
#define DW_EH_PE_pcrel    0x10
#define DW_EH_PE_textrel  0x20
#define DW_EH_PE_datarel  0x30
#define DW_EH_PE_funcrel  0x40
#define DW_EH_PE_aligned  0x50
#define DW_EH_PE_indirect 0x80
#define DW_EH_PE_omit     0xFF

/* Read unsigned LEB128 value from byte stream */
static uint64_t read_uleb128(const uint8_t **p)
{
    uint64_t result = 0;
    unsigned shift = 0;
    uint8_t byte;

    do {
        byte = **p;
        (*p)++;
        result |= (uint64_t)(byte & 0x7F) << shift;
        shift += 7;
    } while (byte & 0x80);

    return result;
}

/* Read signed LEB128 value from byte stream */
static int64_t read_sleb128(const uint8_t **p)
{
    int64_t result = 0;
    unsigned shift = 0;
    uint8_t byte;

    do {
        byte = **p;
        (*p)++;
        result |= (int64_t)(byte & 0x7F) << shift;
        shift += 7;
    } while (byte & 0x80);

    /* Sign extend */
    if (shift < 64 && (byte & 0x40))
        result |= -(((int64_t)1) << shift);

    return result;
}

/* Read an encoded pointer value */
static uint64_t read_encoded_ptr(const uint8_t **p, uint8_t encoding,
                                 uint64_t base)
{
    if (encoding == DW_EH_PE_omit)
        return 0;

    const uint8_t *start = *p;
    uint64_t result = 0;

    /* Read the value */
    switch (encoding & 0x0F) {
    case DW_EH_PE_absptr: {
        uint64_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = val;
        break;
    }
    case DW_EH_PE_uleb128:
        result = read_uleb128(p);
        break;
    case DW_EH_PE_udata2: {
        uint16_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = val;
        break;
    }
    case DW_EH_PE_udata4: {
        uint32_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = val;
        break;
    }
    case DW_EH_PE_udata8: {
        uint64_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = val;
        break;
    }
    case DW_EH_PE_sleb128:
        result = (uint64_t)read_sleb128(p);
        break;
    case DW_EH_PE_sdata2: {
        int16_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = (uint64_t)(int64_t)val;
        break;
    }
    case DW_EH_PE_sdata4: {
        int32_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = (uint64_t)(int64_t)val;
        break;
    }
    case DW_EH_PE_sdata8: {
        int64_t val;
        memcpy(&val, *p, sizeof(val));
        *p += sizeof(val);
        result = (uint64_t)val;
        break;
    }
    default:
        return 0;
    }

    /* Apply relocation */
    if (result != 0) {
        switch (encoding & 0x70) {
        case 0:
            break; /* absolute */
        case DW_EH_PE_pcrel:
            result += (uint64_t)(uintptr_t)start;
            break;
        case DW_EH_PE_funcrel:
            result += base;
            break;
        case DW_EH_PE_textrel:
        case DW_EH_PE_datarel:
        case DW_EH_PE_aligned:
            break; /* unsupported, leave as-is */
        }
    }

    /* Indirect: result is a pointer to the actual value */
    if ((encoding & DW_EH_PE_indirect) && result != 0) {
        result = *(uint64_t *)(uintptr_t)result;
    }

    return result;
}

/* ========================================================================= */
/* __gxx_personality_v0 -- GCC C++ personality routine                       */
/* ========================================================================= */

/*
 * The personality routine is called by the unwinder for each frame.
 * It examines the LSDA to determine if this frame has a handler
 * for the exception.
 *
 * In phase 1 (search), it returns _URC_HANDLER_FOUND if a matching
 * catch clause exists.
 *
 * In phase 2 (cleanup), it sets up the context to jump to the
 * landing pad and returns _URC_INSTALL_CONTEXT.
 */
_Unwind_Reason_Code __gxx_personality_v0(
    int version,
    _Unwind_Action actions,
    _Unwind_Exception_Class exception_class,
    struct _Unwind_Exception *ue_header,
    struct _Unwind_Context *context)
{
    if (version != 1)
        return _URC_FATAL_PHASE1_ERROR;

    /* Get LSDA for this frame */
    const uint8_t *lsda =
        (const uint8_t *)(uintptr_t)_Unwind_GetLanguageSpecificData(context);

    if (!lsda)
        return _URC_CONTINUE_UNWIND;

    uint64_t func_start = _Unwind_GetRegionStart(context);
    uint64_t ip = _Unwind_GetIP(context) - 1; /* -1 to get call site */

    /* ---- Parse LSDA header ---- */
    const uint8_t *p = lsda;

    /* Landing pad base encoding and value */
    uint8_t lp_start_encoding = *p++;
    uint64_t lp_start = func_start;
    if (lp_start_encoding != DW_EH_PE_omit)
        lp_start = read_encoded_ptr(&p, lp_start_encoding, func_start);

    /* Type table encoding */
    uint8_t tt_encoding = *p++;
    const uint8_t *type_table = (void *)0;

    if (tt_encoding != DW_EH_PE_omit) {
        uint64_t tt_offset = read_uleb128(&p);
        type_table = p + tt_offset;
    }

    /* Call site table encoding and length */
    uint8_t cs_encoding = *p++;
    uint64_t cs_length = read_uleb128(&p);

    const uint8_t *cs_table = p;
    const uint8_t *cs_table_end = p + cs_length;
    const uint8_t *action_table = cs_table_end;

    /* ---- Search call site table ---- */
    uint64_t landing_pad = 0;
    int64_t action_offset = 0;
    int found_cs = 0;

    p = cs_table;
    while (p < cs_table_end) {
        uint64_t cs_start  = read_encoded_ptr(&p, cs_encoding, func_start);
        uint64_t cs_len    = read_encoded_ptr(&p, cs_encoding, func_start);
        uint64_t cs_lp     = read_encoded_ptr(&p, cs_encoding, func_start);
        uint64_t cs_action = read_uleb128(&p);

        /* Check if IP falls within this call site */
        if (ip >= func_start + cs_start &&
            ip <  func_start + cs_start + cs_len) {
            if (cs_lp != 0)
                landing_pad = lp_start + cs_lp;
            action_offset = (int64_t)cs_action;
            found_cs = 1;
            break;
        }
    }

    if (!found_cs)
        return _URC_CONTINUE_UNWIND;

    /* No landing pad for this call site */
    if (landing_pad == 0)
        return _URC_CONTINUE_UNWIND;

    /* ---- Process action records ---- */
    int handler_switch_value = 0;
    int found_handler = 0;
    int found_cleanup = 0;

    if (action_offset > 0) {
        const uint8_t *ap = action_table + action_offset - 1;

        while (1) {
            int64_t type_filter = read_sleb128(&ap);
            const uint8_t *ap_saved = ap;
            int64_t next_offset = read_sleb128(&ap);

            if (type_filter > 0) {
                /*
                 * Positive filter: this is a catch clause.
                 * type_filter is an index into the type table.
                 *
                 * For now, we treat any positive filter as a match.
                 * A full implementation would compare the exception's
                 * type_info against the type table entry.
                 */
                if (exception_class == CXX_EXCEPTION_CLASS) {
                    /* Match: this catch handler catches our exception */
                    handler_switch_value = (int)type_filter;
                    found_handler = 1;
                    break;
                }
            } else if (type_filter == 0) {
                /* Cleanup action (no type filter) */
                found_cleanup = 1;
            }
            /* Negative filter: exception spec (not implemented) */

            if (next_offset == 0)
                break;

            ap = ap_saved + next_offset;
        }
    } else if (landing_pad != 0) {
        /* action_offset == 0 means cleanup-only landing pad */
        found_cleanup = 1;
    }

    /* ---- Phase 1: just report whether we found a handler ---- */
    if (actions & _UA_SEARCH_PHASE) {
        if (found_handler)
            return _URC_HANDLER_FOUND;
        return _URC_CONTINUE_UNWIND;
    }

    /* ---- Phase 2: install context for landing pad ---- */
    if (actions & _UA_CLEANUP_PHASE) {
        if (found_handler || found_cleanup) {
            /*
             * Set up the registers that the landing pad expects:
             *   GR[0] (RAX on x86_64): pointer to _Unwind_Exception
             *   GR[1] (RDX on x86_64): handler switch value (type selector)
             */
            _Unwind_SetGR(context, 0, (uint64_t)(uintptr_t)ue_header);
            _Unwind_SetGR(context, 1, (uint64_t)handler_switch_value);
            _Unwind_SetIP(context, landing_pad);
            return _URC_INSTALL_CONTEXT;
        }
        return _URC_CONTINUE_UNWIND;
    }

    return _URC_CONTINUE_UNWIND;
}
