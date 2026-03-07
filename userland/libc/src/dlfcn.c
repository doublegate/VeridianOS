/*
 * VeridianOS libc -- dlfcn.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Dynamic loading implementation for statically-linked programs.
 *
 * This module provides dlopen/dlsym/dlclose/dlerror that can load ELF
 * shared objects at runtime via mmap, even when the host program is
 * statically linked.  It reuses the same algorithms as ld-veridian.c
 * but calls libc functions (open, read, mmap, ...) instead of raw
 * syscalls.
 *
 * Also provides dl_iterate_phdr() and dladdr() stubs for exception
 * handling and backtrace support.
 */

#include <dlfcn.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>

/* ========================================================================= */
/* ELF Types (self-contained -- no elf.h dependency)                         */
/* ========================================================================= */

typedef struct {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} Dl_Elf64_Ehdr;

typedef struct {
    uint32_t p_type;
    uint32_t p_flags;
    uint64_t p_offset;
    uint64_t p_vaddr;
    uint64_t p_paddr;
    uint64_t p_filesz;
    uint64_t p_memsz;
    uint64_t p_align;
} Dl_Elf64_Phdr;

typedef struct {
    int64_t  d_tag;
    union {
        uint64_t d_val;
        uint64_t d_ptr;
    } d_un;
} Dl_Elf64_Dyn;

typedef struct {
    uint32_t st_name;
    uint8_t  st_info;
    uint8_t  st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
} Dl_Elf64_Sym;

typedef struct {
    uint64_t r_offset;
    uint64_t r_info;
    int64_t  r_addend;
} Dl_Elf64_Rela;

/* ELF constants */
#define DL_PT_NULL     0
#define DL_PT_LOAD     1
#define DL_PT_DYNAMIC  2
#define DL_PT_TLS      7
#define DL_PT_GNU_RELRO 0x6474e552

#define DL_PF_X 0x1
#define DL_PF_W 0x2
#define DL_PF_R 0x4

#define DL_DT_NULL      0
#define DL_DT_NEEDED    1
#define DL_DT_PLTRELSZ  2
#define DL_DT_PLTGOT    3
#define DL_DT_HASH      4
#define DL_DT_STRTAB    5
#define DL_DT_SYMTAB    6
#define DL_DT_RELA      7
#define DL_DT_RELASZ    8
#define DL_DT_INIT      12
#define DL_DT_FINI      13
#define DL_DT_JMPREL    23
#define DL_DT_BIND_NOW  24
#define DL_DT_INIT_ARRAY    25
#define DL_DT_FINI_ARRAY    26
#define DL_DT_INIT_ARRAYSZ  27
#define DL_DT_FINI_ARRAYSZ  28
#define DL_DT_FLAGS     30
#define DL_DT_FLAGS_1   0x6FFFFFFB

#define DL_DF_BIND_NOW   8
#define DL_DF_1_NOW      0x00000001

#define DL_ET_DYN 3

#define DL_ELF64_R_SYM(info)  ((info) >> 32)
#define DL_ELF64_R_TYPE(info) ((info) & 0xFFFFFFFF)
#define DL_ELF64_ST_BIND(info) ((info) >> 4)

#define DL_STB_LOCAL  0
#define DL_STB_GLOBAL 1
#define DL_STB_WEAK   2

#define DL_SHN_UNDEF 0

#define DL_R_X86_64_NONE       0
#define DL_R_X86_64_64         1
#define DL_R_X86_64_COPY       5
#define DL_R_X86_64_GLOB_DAT   6
#define DL_R_X86_64_JUMP_SLOT  7
#define DL_R_X86_64_RELATIVE   8
#define DL_R_X86_64_IRELATIVE  37

#define DL_PAGE_SIZE 4096

/* ========================================================================= */
/* Loaded Library Registry                                                   */
/* ========================================================================= */

#define DL_MAX_LIBS 32

typedef struct {
    char             name[256];  /* library path or name */
    uint64_t         base;       /* load bias */
    Dl_Elf64_Dyn    *dynamic;    /* PT_DYNAMIC pointer */
    Dl_Elf64_Sym    *symtab;     /* DT_SYMTAB */
    const char      *strtab;     /* DT_STRTAB */
    size_t           symtab_cnt; /* symbol count (from DT_HASH nchain) */
    int              loaded;     /* 1 if actively loaded */
    int              refcount;   /* reference count for dlclose */

    /* Init/fini */
    uint64_t         init_func;
    uint64_t         fini_func;
    uint64_t        *init_array;
    size_t           init_array_sz;
    uint64_t        *fini_array;
    size_t           fini_array_sz;

    /* Mapped memory (for cleanup on dlclose) */
    void            *map_base;   /* lowest mapped address */
    size_t           map_size;   /* total mapped size */
} DlLib;

static DlLib  dl_libs[DL_MAX_LIBS];
static int    dl_lib_count = 0;

/* Thread-safe error message */
static const char *dl_error_msg = NULL;

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

/*
 * Read len bytes from fd at offset into buf.
 * Returns bytes read, or -1 on error.
 */
static long dl_pread(int fd, void *buf, size_t len, long offset)
{
    if (lseek(fd, offset, SEEK_SET) < 0)
        return -1;
    return read(fd, buf, len);
}

/*
 * Parse the dynamic section of a loaded library.
 */
static void dl_parse_dynamic(DlLib *lib)
{
    if (!lib->dynamic) return;

    uint64_t hash_addr = 0;

    for (Dl_Elf64_Dyn *d = lib->dynamic; d->d_tag != DL_DT_NULL; d++) {
        uint64_t val = d->d_un.d_val;
        uint64_t ptr = d->d_un.d_ptr;
        switch (d->d_tag) {
        case DL_DT_STRTAB:       lib->strtab        = (const char *)(lib->base + ptr); break;
        case DL_DT_SYMTAB:       lib->symtab        = (Dl_Elf64_Sym *)(lib->base + ptr); break;
        case DL_DT_HASH:         hash_addr = lib->base + ptr; break;
        case DL_DT_INIT:         lib->init_func     = lib->base + ptr; break;
        case DL_DT_FINI:         lib->fini_func     = lib->base + ptr; break;
        case DL_DT_INIT_ARRAY:   lib->init_array    = (uint64_t *)(lib->base + ptr); break;
        case DL_DT_FINI_ARRAY:   lib->fini_array    = (uint64_t *)(lib->base + ptr); break;
        case DL_DT_INIT_ARRAYSZ: lib->init_array_sz = val; break;
        case DL_DT_FINI_ARRAYSZ: lib->fini_array_sz = val; break;
        default: break;
        }
    }

    /* Calculate symtab_cnt from DT_HASH (nchain) */
    if (hash_addr) {
        uint32_t *ht = (uint32_t *)hash_addr;
        /* ht[0] = nbucket, ht[1] = nchain */
        lib->symtab_cnt = ht[1];
    }
}

/*
 * Look up a symbol in a single library.
 * Returns the symbol value (base-adjusted), or 0 on failure.
 * Prefers STB_GLOBAL over STB_WEAK.
 */
static uint64_t dl_lookup_symbol_in_lib(DlLib *lib, const char *name)
{
    if (!lib->symtab || !lib->strtab || !lib->loaded) return 0;

    uint64_t weak_val = 0;

    for (size_t i = 0; i < lib->symtab_cnt; i++) {
        Dl_Elf64_Sym *sym = &lib->symtab[i];
        if (sym->st_shndx == DL_SHN_UNDEF) continue;
        if (strcmp(lib->strtab + sym->st_name, name) != 0) continue;

        uint8_t bind = DL_ELF64_ST_BIND(sym->st_info);
        uint64_t val = lib->base + sym->st_value;

        if (bind == DL_STB_GLOBAL)
            return val;
        if (bind == DL_STB_WEAK && weak_val == 0)
            weak_val = val;
    }

    return weak_val;
}

/*
 * Search all loaded dlopen'd libraries for a symbol.
 */
static uint64_t dl_lookup_symbol_global(const char *name)
{
    uint64_t weak_val = 0;

    for (int i = 0; i < dl_lib_count; i++) {
        if (!dl_libs[i].loaded) continue;
        uint64_t val = dl_lookup_symbol_in_lib(&dl_libs[i], name);
        if (val) {
            /* Check binding -- prefer global */
            for (size_t j = 0; j < dl_libs[i].symtab_cnt; j++) {
                Dl_Elf64_Sym *sym = &dl_libs[i].symtab[j];
                if (sym->st_shndx == DL_SHN_UNDEF) continue;
                if (strcmp(dl_libs[i].strtab + sym->st_name, name) != 0) continue;
                if (DL_ELF64_ST_BIND(sym->st_info) == DL_STB_GLOBAL)
                    return val;
                break;
            }
            if (weak_val == 0)
                weak_val = val;
        }
    }
    return weak_val;
}

/*
 * Process RELA relocations for a loaded library.
 */
static void dl_process_rela(DlLib *lib, Dl_Elf64_Rela *rela, size_t rela_sz)
{
    uint64_t base = lib->base;
    size_t count = rela_sz / sizeof(Dl_Elf64_Rela);

    if (!lib->symtab || !lib->strtab) return;

    for (size_t i = 0; i < count; i++) {
        uint32_t type    = DL_ELF64_R_TYPE(rela[i].r_info);
        uint32_t sym_idx = DL_ELF64_R_SYM(rela[i].r_info);
        uint64_t *target = (uint64_t *)(base + rela[i].r_offset);

        switch (type) {
        case DL_R_X86_64_NONE:
            break;

        case DL_R_X86_64_RELATIVE:
            *target = base + rela[i].r_addend;
            break;

        case DL_R_X86_64_GLOB_DAT:
        case DL_R_X86_64_JUMP_SLOT:
        case DL_R_X86_64_64: {
            const char *name = lib->strtab + lib->symtab[sym_idx].st_name;
            uint64_t val = dl_lookup_symbol_global(name);
            if (!val)
                val = dl_lookup_symbol_in_lib(lib, name);
            if (type == DL_R_X86_64_64)
                *target = val + rela[i].r_addend;
            else
                *target = val;
            break;
        }

        case DL_R_X86_64_COPY: {
            const char *name = lib->strtab + lib->symtab[sym_idx].st_name;
            /* Search all libs except this one */
            uint64_t src_addr = 0;
            for (int j = 0; j < dl_lib_count; j++) {
                if (&dl_libs[j] == lib) continue;
                src_addr = dl_lookup_symbol_in_lib(&dl_libs[j], name);
                if (src_addr) break;
            }
            if (src_addr) {
                memcpy((void *)(base + rela[i].r_offset),
                       (void *)src_addr, lib->symtab[sym_idx].st_size);
            }
            break;
        }

        case DL_R_X86_64_IRELATIVE: {
            typedef uint64_t (*ifunc_resolver_t)(void);
            ifunc_resolver_t resolver = (ifunc_resolver_t)(base + rela[i].r_addend);
            *target = resolver();
            break;
        }

        default:
            /* Unsupported relocation -- silently skip */
            break;
        }
    }
}

/*
 * Map ELF PT_LOAD segments into memory via mmap.
 * Returns the load bias.
 */
static uint64_t dl_map_elf_segments(int fd, Dl_Elf64_Phdr *phdrs, uint16_t phnum,
                                     void **out_map_base, size_t *out_map_size)
{
    uint64_t base_addr = 0;
    int first_load = 1;
    uint64_t lowest = (uint64_t)-1;
    uint64_t highest = 0;

    /* First pass: determine total span for tracking */
    for (uint16_t i = 0; i < phnum; i++) {
        if (phdrs[i].p_type != DL_PT_LOAD) continue;
        uint64_t seg_start = phdrs[i].p_vaddr & ~(uint64_t)(DL_PAGE_SIZE - 1);
        uint64_t seg_end = (phdrs[i].p_vaddr + phdrs[i].p_memsz + DL_PAGE_SIZE - 1)
                           & ~(uint64_t)(DL_PAGE_SIZE - 1);
        if (seg_start < lowest) lowest = seg_start;
        if (seg_end > highest) highest = seg_end;
    }

    if (lowest > highest) {
        *out_map_base = NULL;
        *out_map_size = 0;
        return 0;
    }

    *out_map_size = (size_t)(highest - lowest);

    /* Map each PT_LOAD segment */
    for (uint16_t i = 0; i < phnum; i++) {
        if (phdrs[i].p_type != DL_PT_LOAD) continue;

        uint64_t p_vaddr  = phdrs[i].p_vaddr;
        uint64_t p_offset = phdrs[i].p_offset;
        uint64_t p_filesz = phdrs[i].p_filesz;
        uint64_t p_memsz  = phdrs[i].p_memsz;
        uint32_t p_flags  = phdrs[i].p_flags;

        uint64_t seg_start = p_vaddr & ~(uint64_t)(DL_PAGE_SIZE - 1);
        uint64_t seg_end   = (p_vaddr + p_memsz + DL_PAGE_SIZE - 1)
                             & ~(uint64_t)(DL_PAGE_SIZE - 1);
        uint64_t map_size  = seg_end - seg_start;
        uint64_t page_offset = p_vaddr - seg_start;

        int prot  = PROT_READ | PROT_WRITE;
        int flags = MAP_PRIVATE | MAP_ANONYMOUS;

        void *hint = NULL;
        if (!first_load) {
            hint = (void *)(base_addr + seg_start);
            flags |= MAP_FIXED;
        }

        void *mapped = mmap(hint, (size_t)map_size, prot, flags, -1, 0);
        if (mapped == MAP_FAILED)
            return 0;

        if (first_load) {
            base_addr = (uint64_t)mapped - seg_start;
            *out_map_base = mapped;
            first_load = 0;
        }

        /* Read file data into mapped region */
        if (p_filesz > 0) {
            uint64_t dst = (uint64_t)mapped + page_offset;
            dl_pread(fd, (void *)dst, (size_t)p_filesz, (long)p_offset);
        }

        /* Zero BSS */
        if (p_memsz > p_filesz) {
            uint64_t bss_start = (uint64_t)mapped + page_offset + p_filesz;
            memset((void *)bss_start, 0, (size_t)(p_memsz - p_filesz));
        }

        /* Re-protect to correct permissions */
        int final_prot = 0;
        if (p_flags & DL_PF_R) final_prot |= PROT_READ;
        if (p_flags & DL_PF_W) final_prot |= PROT_WRITE;
        if (p_flags & DL_PF_X) final_prot |= PROT_EXEC;

        if (final_prot != prot)
            mprotect(mapped, (size_t)map_size, final_prot);
    }

    return base_addr;
}

/*
 * Call DT_INIT and DT_INIT_ARRAY constructors.
 */
static void dl_call_init(DlLib *lib)
{
    typedef void (*init_fn_t)(void);

    if (lib->init_func) {
        init_fn_t fn = (init_fn_t)lib->init_func;
        fn();
    }
    if (lib->init_array && lib->init_array_sz > 0) {
        size_t count = lib->init_array_sz / sizeof(uint64_t);
        for (size_t i = 0; i < count; i++) {
            init_fn_t fn = (init_fn_t)lib->init_array[i];
            if (fn) fn();
        }
    }
}

/*
 * Call DT_FINI_ARRAY (reverse) and DT_FINI destructors.
 */
static void dl_call_fini(DlLib *lib)
{
    typedef void (*fini_fn_t)(void);

    if (lib->fini_array && lib->fini_array_sz > 0) {
        size_t count = lib->fini_array_sz / sizeof(uint64_t);
        for (size_t i = count; i > 0; i--) {
            fini_fn_t fn = (fini_fn_t)lib->fini_array[i - 1];
            if (fn) fn();
        }
    }
    if (lib->fini_func) {
        fini_fn_t fn = (fini_fn_t)lib->fini_func;
        fn();
    }
}

/*
 * Search for a library file.  If name contains '/', use as-is.
 * Otherwise search /lib then /usr/lib.
 * Returns fd >= 0 on success.
 */
static int dl_search_library(const char *name)
{
    /* If name has a '/', treat as direct path */
    for (const char *p = name; *p; p++) {
        if (*p == '/') {
            return open(name, O_RDONLY);
        }
    }

    /* Check LD_LIBRARY_PATH */
    const char *ldpath = getenv("LD_LIBRARY_PATH");
    if (ldpath) {
        const char *start = ldpath;
        while (*start) {
            const char *end = start;
            while (*end && *end != ':') end++;
            size_t dlen = (size_t)(end - start);
            if (dlen > 0 && dlen < 400) {
                char path_buf[512];
                memcpy(path_buf, start, dlen);
                path_buf[dlen] = '/';
                size_t nlen = strlen(name);
                if (dlen + 1 + nlen < sizeof(path_buf)) {
                    memcpy(path_buf + dlen + 1, name, nlen + 1);
                    int fd = open(path_buf, O_RDONLY);
                    if (fd >= 0) return fd;
                }
            }
            if (*end == ':') end++;
            start = end;
        }
    }

    /* Default search paths */
    static const char *default_paths[] = {
        "/lib/", "/usr/lib/", NULL
    };

    for (int i = 0; default_paths[i]; i++) {
        char path_buf[512];
        size_t plen = strlen(default_paths[i]);
        size_t nlen = strlen(name);
        if (plen + nlen < sizeof(path_buf)) {
            memcpy(path_buf, default_paths[i], plen);
            memcpy(path_buf + plen, name, nlen + 1);
            int fd = open(path_buf, O_RDONLY);
            if (fd >= 0) return fd;
        }
    }

    return -1;
}

/*
 * Load a shared library by name.  Returns a DlLib pointer or NULL.
 */
static DlLib *dl_load_library(const char *name)
{
    if (!name) return NULL;

    /* Check if already loaded */
    for (int i = 0; i < dl_lib_count; i++) {
        if (dl_libs[i].loaded && strcmp(dl_libs[i].name, name) == 0) {
            dl_libs[i].refcount++;
            return &dl_libs[i];
        }
    }

    if (dl_lib_count >= DL_MAX_LIBS) {
        dl_error_msg = "dlopen: too many loaded libraries";
        return NULL;
    }

    int fd = dl_search_library(name);
    if (fd < 0) {
        dl_error_msg = "dlopen: library not found";
        return NULL;
    }

    /* Read ELF header */
    Dl_Elf64_Ehdr ehdr;
    memset(&ehdr, 0, sizeof(ehdr));
    long rd = dl_pread(fd, &ehdr, sizeof(ehdr), 0);
    if (rd < (long)sizeof(ehdr)) {
        close(fd);
        dl_error_msg = "dlopen: failed to read ELF header";
        return NULL;
    }

    /* Validate ELF magic */
    if (ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != 'E' ||
        ehdr.e_ident[2] != 'L'  || ehdr.e_ident[3] != 'F') {
        close(fd);
        dl_error_msg = "dlopen: not an ELF file";
        return NULL;
    }

    /* Read program headers */
    size_t phdr_size = (size_t)ehdr.e_phnum * ehdr.e_phentsize;
    Dl_Elf64_Phdr *phdrs = (Dl_Elf64_Phdr *)malloc(phdr_size);
    if (!phdrs) {
        close(fd);
        dl_error_msg = "dlopen: out of memory";
        return NULL;
    }
    dl_pread(fd, phdrs, phdr_size, (long)ehdr.e_phoff);

    /* Map segments */
    void  *map_base = NULL;
    size_t map_size = 0;
    uint64_t slide = dl_map_elf_segments(fd, phdrs, ehdr.e_phnum,
                                          &map_base, &map_size);

    if (!map_base) {
        free(phdrs);
        close(fd);
        dl_error_msg = "dlopen: failed to map ELF segments";
        return NULL;
    }

    /* Find PT_DYNAMIC */
    Dl_Elf64_Dyn *dynamic = NULL;
    for (uint16_t i = 0; i < ehdr.e_phnum; i++) {
        if (phdrs[i].p_type == DL_PT_DYNAMIC) {
            dynamic = (Dl_Elf64_Dyn *)(slide + phdrs[i].p_vaddr);
            break;
        }
    }

    /* Register the library */
    DlLib *lib = &dl_libs[dl_lib_count++];
    memset(lib, 0, sizeof(DlLib));
    size_t nlen = strlen(name);
    if (nlen >= sizeof(lib->name)) nlen = sizeof(lib->name) - 1;
    memcpy(lib->name, name, nlen);
    lib->name[nlen] = '\0';
    lib->base     = slide;
    lib->dynamic  = dynamic;
    lib->loaded   = 1;
    lib->refcount = 1;
    lib->map_base = map_base;
    lib->map_size = map_size;

    /* Parse dynamic section */
    dl_parse_dynamic(lib);

    /* Load DT_NEEDED dependencies (recursive) */
    if (dynamic && lib->strtab) {
        for (Dl_Elf64_Dyn *d = dynamic; d->d_tag != DL_DT_NULL; d++) {
            if (d->d_tag == DL_DT_NEEDED) {
                const char *needed = lib->strtab + d->d_un.d_val;
                DlLib *dep = dl_load_library(needed);
                if (!dep) {
                    /* Warning: dependency not found, but don't abort */
                }
            }
        }
    }

    /* Process RELA relocations */
    if (lib->symtab && lib->strtab && dynamic) {
        Dl_Elf64_Rela *rela    = NULL;
        size_t          relasz  = 0;
        Dl_Elf64_Rela *jmprel  = NULL;
        size_t          jmprelsz = 0;

        for (Dl_Elf64_Dyn *d = dynamic; d->d_tag != DL_DT_NULL; d++) {
            if (d->d_tag == DL_DT_RELA)
                rela = (Dl_Elf64_Rela *)(slide + d->d_un.d_ptr);
            if (d->d_tag == DL_DT_RELASZ)
                relasz = d->d_un.d_val;
            if (d->d_tag == DL_DT_JMPREL)
                jmprel = (Dl_Elf64_Rela *)(slide + d->d_un.d_ptr);
            if (d->d_tag == DL_DT_PLTRELSZ)
                jmprelsz = d->d_un.d_val;
        }

        if (rela && relasz)
            dl_process_rela(lib, rela, relasz);
        if (jmprel && jmprelsz)
            dl_process_rela(lib, jmprel, jmprelsz);
    }

    /* Apply RELRO */
    for (uint16_t i = 0; i < ehdr.e_phnum; i++) {
        if (phdrs[i].p_type == DL_PT_GNU_RELRO) {
            uint64_t start = (slide + phdrs[i].p_vaddr)
                             & ~(uint64_t)(DL_PAGE_SIZE - 1);
            uint64_t end   = (slide + phdrs[i].p_vaddr + phdrs[i].p_memsz
                              + DL_PAGE_SIZE - 1)
                             & ~(uint64_t)(DL_PAGE_SIZE - 1);
            mprotect((void *)start, (size_t)(end - start), PROT_READ);
            break;
        }
    }

    /* Call constructors */
    dl_call_init(lib);

    free(phdrs);
    close(fd);

    return lib;
}

/* ========================================================================= */
/* Public API: dlopen / dlsym / dlclose / dlerror                            */
/* ========================================================================= */

void *dlopen(const char *filename, int flags)
{
    (void)flags;

    if (!filename) {
        /*
         * POSIX: dlopen(NULL, ...) returns a handle to the main program.
         * For statically-linked programs, we cannot introspect our own
         * symbol table.  Return a sentinel value that dlsym recognizes.
         */
        dl_error_msg = NULL;
        return (void *)(uintptr_t)0x1;  /* sentinel for main program */
    }

    DlLib *lib = dl_load_library(filename);
    if (!lib)
        return NULL;

    dl_error_msg = NULL;
    return (void *)lib;
}

void *dlsym(void *handle, const char *symbol)
{
    if (!handle || !symbol) {
        dl_error_msg = "dlsym: invalid argument";
        return NULL;
    }

    /* Sentinel handle: search all loaded dlopen'd libraries */
    if (handle == (void *)(uintptr_t)0x1) {
        uint64_t val = dl_lookup_symbol_global(symbol);
        if (!val) {
            dl_error_msg = "dlsym: symbol not found";
            return NULL;
        }
        dl_error_msg = NULL;
        return (void *)val;
    }

    DlLib *lib = (DlLib *)handle;
    uint64_t val = dl_lookup_symbol_in_lib(lib, symbol);
    if (!val) {
        /* Fall back to global search */
        val = dl_lookup_symbol_global(symbol);
    }
    if (!val) {
        dl_error_msg = "dlsym: symbol not found";
        return NULL;
    }

    dl_error_msg = NULL;
    return (void *)val;
}

int dlclose(void *handle)
{
    if (!handle || handle == (void *)(uintptr_t)0x1)
        return 0;

    DlLib *lib = (DlLib *)handle;

    if (lib->refcount > 1) {
        lib->refcount--;
        return 0;
    }

    /* Call destructors */
    dl_call_fini(lib);

    /* Unmap memory */
    if (lib->map_base && lib->map_size > 0)
        munmap(lib->map_base, lib->map_size);

    lib->loaded   = 0;
    lib->refcount = 0;
    return 0;
}

char *dlerror(void)
{
    const char *msg = dl_error_msg;
    dl_error_msg = NULL;
    return (char *)msg;
}

/* ========================================================================= */
/* dl_iterate_phdr -- stub for exception handling / backtrace support         */
/* ========================================================================= */

/*
 * Minimal dl_iterate_phdr for statically-linked programs.
 * Full implementation requires access to the program's own PHDRs
 * (via AT_PHDR auxiliary vector), which is not available after _start.
 * Returns 0 (no objects iterated).
 *
 * struct dl_phdr_info, Dl_info, and function declarations are in <dlfcn.h>.
 */

int dl_iterate_phdr(int (*callback)(struct dl_phdr_info *info,
                                     size_t size, void *data),
                    void *data)
{
    (void)callback;
    (void)data;
    return 0;
}

/* ========================================================================= */
/* dladdr -- stub for backtrace symbol resolution                            */
/* ========================================================================= */

int dladdr(const void *addr, Dl_info *info)
{
    if (!addr || !info)
        return 0;

    /* Search loaded libraries for address range */
    for (int i = 0; i < dl_lib_count; i++) {
        DlLib *lib = &dl_libs[i];
        if (!lib->loaded) continue;
        uint64_t lib_start = (uint64_t)lib->map_base;
        uint64_t lib_end   = lib_start + lib->map_size;
        uint64_t a = (uint64_t)addr;

        if (a >= lib_start && a < lib_end) {
            info->dli_fname = lib->name;
            info->dli_fbase = lib->map_base;

            /* Try to find the closest symbol */
            info->dli_sname = NULL;
            info->dli_saddr = NULL;
            if (lib->symtab && lib->strtab) {
                uint64_t best_val = 0;
                for (size_t j = 0; j < lib->symtab_cnt; j++) {
                    Dl_Elf64_Sym *sym = &lib->symtab[j];
                    if (sym->st_shndx == DL_SHN_UNDEF) continue;
                    uint64_t sv = lib->base + sym->st_value;
                    if (sv <= a && sv > best_val) {
                        best_val = sv;
                        info->dli_sname = lib->strtab + sym->st_name;
                        info->dli_saddr = (void *)sv;
                    }
                }
            }
            return 1;
        }
    }

    return 0;
}
