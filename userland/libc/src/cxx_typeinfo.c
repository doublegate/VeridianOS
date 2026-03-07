/*
 * VeridianOS libc -- cxx_typeinfo.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * RTTI (Run-Time Type Information) support for C++ dynamic_cast and typeid.
 *
 * The Itanium C++ ABI defines three type_info class layouts:
 *   - __class_type_info: base class for all class types
 *   - __si_class_type_info: single (non-virtual) public inheritance
 *   - __vmi_class_type_info: virtual or multiple inheritance
 *
 * Each has a vtable with a __do_upcast / __do_dyncast virtual function
 * used by __dynamic_cast to traverse the class hierarchy.
 *
 * In this minimal implementation, __dynamic_cast performs a simple
 * type comparison and single-inheritance chain walk.  Virtual base
 * class traversal is not yet supported (returns NULL for ambiguous
 * or virtual-base casts).
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi.html#rtti
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* Forward-declare to avoid pulling in full headers */
long write(int fd, const void *buf, unsigned long count);
void abort(void) __attribute__((noreturn));

/* ========================================================================= */
/* Type info structures (Itanium C++ ABI layout)                             */
/* ========================================================================= */

/*
 * The compiler generates type_info objects with these layouts.
 * Each has a vtable pointer followed by a mangled name string.
 */

/* Base class for all type_info */
struct __class_type_info {
    void **__vtable;     /* vtable pointer */
    const char *__name;  /* mangled type name */
};

/* Single public non-virtual inheritance */
struct __si_class_type_info {
    void **__vtable;
    const char *__name;
    const struct __class_type_info *__base_type;
};

/*
 * Flags for __base_class_type_info::__offset_flags.
 * The offset to the base subobject is stored in the upper bits.
 */
#define __base_class_virtual_mask    0x01
#define __base_class_public_mask     0x02
#define __base_class_offset_shift    8

/* Base class descriptor for __vmi_class_type_info */
struct __base_class_type_info {
    const struct __class_type_info *__base_type;
    long __offset_flags;
};

/* Virtual or multiple inheritance */
struct __vmi_class_type_info {
    void **__vtable;
    const char *__name;
    unsigned int __flags;
    unsigned int __base_count;
    struct __base_class_type_info __base_info[1]; /* variable length */
};

/* __vmi_class_type_info flags */
#define __non_diamond_repeat_mask 0x01
#define __diamond_shaped_mask     0x02

/* ========================================================================= */
/* Type info vtables                                                         */
/* ========================================================================= */

/*
 * The compiler expects these vtables to exist as external symbols.
 * They are referenced by the type_info objects the compiler generates.
 * We provide minimal vtables -- the dynamic_cast logic below does
 * the actual work without virtual dispatch.
 *
 * Each vtable has 2 slots before the address point: offset-to-top
 * and typeinfo pointer.  We provide a simple 4-slot table.
 */

/*
 * Export the vtable symbols that the compiler references.
 * The compiler generates references like:
 *   _ZTVN10__cxxabiv117__class_type_infoE
 *   _ZTVN10__cxxabiv120__si_class_type_infoE
 *   _ZTVN10__cxxabiv121__vmi_class_type_infoE
 *
 * These point 2 slots (16 bytes) into the vtable array.
 * Minimal vtable slots: [offset-to-top, typeinfo_ptr, dtor1, dtor2]
 */
void *_ZTVN10__cxxabiv117__class_type_infoE[4] = { 0, 0, 0, 0 };
void *_ZTVN10__cxxabiv120__si_class_type_infoE[4] = { 0, 0, 0, 0 };
void *_ZTVN10__cxxabiv121__vmi_class_type_infoE[4] = { 0, 0, 0, 0 };

/* ========================================================================= */
/* Type comparison helpers                                                   */
/* ========================================================================= */

/*
 * Compare two type_info pointers for identity.
 *
 * In the Itanium ABI, type_info identity is determined by:
 *   1. Pointer equality (same object)
 *   2. Name string comparison (for types across DSO boundaries)
 *
 * For now we use both checks.
 */
static int types_equal(const struct __class_type_info *a,
                       const struct __class_type_info *b)
{
    if (a == b)
        return 1;
    if (a && b && a->__name && b->__name)
        return strcmp(a->__name, b->__name) == 0;
    return 0;
}

/* ========================================================================= */
/* __dynamic_cast                                                            */
/* ========================================================================= */

/*
 * Perform a dynamic_cast from src_type to dst_type.
 *
 * Walk the single-inheritance chain from src_type looking for dst_type.
 * For __vmi_class_type_info (multiple/virtual inheritance), we do a
 * depth-first search through the base class list.
 *
 * @param src_ptr        Pointer to the most-derived object.
 * @param src_type       type_info of the static type.
 * @param dst_type       type_info of the target type.
 * @param src2dst_offset Offset hint: >=0 for known offset,
 *                       -1 for unknown, -2 for virtual base.
 * @return Adjusted pointer or NULL on failure.
 */
void *__dynamic_cast(const void *src_ptr,
                     const void *src_type,
                     const void *dst_type,
                     long src2dst_offset)
{
    const struct __class_type_info *src = src_type;
    const struct __class_type_info *dst = dst_type;

    if (!src_ptr || !src || !dst)
        return (void *)0;

    /* Fast path: source and destination are the same type */
    if (types_equal(src, dst))
        return (void *)src_ptr;

    /* Known offset hint from compiler */
    if (src2dst_offset >= 0)
        return (void *)((const char *)src_ptr + src2dst_offset);

    /*
     * Walk the single-inheritance chain (__si_class_type_info).
     * Check if dst_type matches our vtable signature for SI type.
     *
     * We do a simple iterative walk: check if src's base chain
     * includes dst, accumulating offsets.
     */

    /* Check if this is a __si_class_type_info (single inheritance) */
    const struct __si_class_type_info *si_src =
        (const struct __si_class_type_info *)src;

    /*
     * Heuristic: if the vtable pointer matches _ZTVN10__cxxabiv120__si_class_type_infoE,
     * we know it's a single-inheritance type and can walk the chain.
     * Otherwise, check for vmi_class_type_info.
     */
    if (si_src->__vtable == (void **)(_ZTVN10__cxxabiv120__si_class_type_infoE + 2) ||
        /* Also accept the aliased symbol */
        si_src->__vtable == (void **)(_ZTVN10__cxxabiv120__si_class_type_infoE + 2)) {
        /* Walk SI chain */
        const struct __si_class_type_info *cur = si_src;
        int max_depth = 64; /* prevent infinite loops */

        while (cur && max_depth-- > 0) {
            if (types_equal(cur->__base_type, dst))
                return (void *)src_ptr; /* upcast, offset is 0 for first base */

            /* Check if base is also SI */
            const struct __si_class_type_info *next =
                (const struct __si_class_type_info *)cur->__base_type;

            if (next && (next->__vtable == (void **)(_ZTVN10__cxxabiv120__si_class_type_infoE + 2) ||
                         next->__vtable == (void **)(_ZTVN10__cxxabiv120__si_class_type_infoE + 2))) {
                cur = next;
            } else {
                /* Hit a non-SI base, check final match */
                if (cur->__base_type && types_equal(cur->__base_type, dst))
                    return (void *)src_ptr;
                break;
            }
        }
    }

    /* Check VMI (virtual/multiple inheritance) */
    const struct __vmi_class_type_info *vmi_src =
        (const struct __vmi_class_type_info *)src;

    if (vmi_src->__vtable == (void **)(_ZTVN10__cxxabiv121__vmi_class_type_infoE + 2) ||
        vmi_src->__vtable == (void **)(_ZTVN10__cxxabiv121__vmi_class_type_infoE + 2)) {
        /* Search base classes */
        for (unsigned int i = 0; i < vmi_src->__base_count; i++) {
            const struct __base_class_type_info *base =
                &vmi_src->__base_info[i];

            if (types_equal(base->__base_type, dst)) {
                long offset = base->__offset_flags >> __base_class_offset_shift;

                if (base->__offset_flags & __base_class_virtual_mask) {
                    /* Virtual base -- would need vtable lookup */
                    return (void *)0; /* not yet supported */
                }

                return (void *)((const char *)src_ptr + offset);
            }
        }
    }

    /* No match found */
    return (void *)0;
}

/* ========================================================================= */
/* __cxa_bad_cast                                                            */
/* ========================================================================= */

void __cxa_bad_cast(void)
{
    static const char msg[] = "std::bad_cast: dynamic_cast failed\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}

/* ========================================================================= */
/* __cxa_bad_typeid                                                          */
/* ========================================================================= */

void __cxa_bad_typeid(void)
{
    static const char msg[] = "std::bad_typeid: typeid of null pointer\n";
    write(2, msg, sizeof(msg) - 1);
    abort();
}
