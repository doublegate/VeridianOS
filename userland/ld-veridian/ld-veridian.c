/*
 * ld-veridian.so -- VeridianOS Dynamic Linker
 *
 * Minimal ELF dynamic linker / runtime loader for VeridianOS.
 * Handles PT_INTERP-delegated program loading:
 *   1. Kernel loads this linker at a fixed address
 *   2. Linker maps the program's LOAD segments
 *   3. Processes DT_NEEDED shared libraries
 *   4. Performs relocations (RELA, REL, JMPREL)
 *   5. Transfers control to the program's entry point
 *
 * Supported relocation types (x86_64):
 *   R_X86_64_RELATIVE (8)  -- base + addend
 *   R_X86_64_GLOB_DAT (6)  -- symbol value
 *   R_X86_64_JUMP_SLOT (7) -- PLT lazy binding
 *   R_X86_64_64 (1)        -- S + A (symbol + addend)
 *
 * Library search path: /lib, /usr/lib
 *
 * Build: x86_64-veridian-gcc -nostdlib -shared -o ld-veridian.so ld-veridian.c
 */

#include <stdint.h>
#include <stddef.h>

/* ===== ELF Structures ===== */

typedef uint64_t Elf64_Addr;
typedef uint64_t Elf64_Off;
typedef uint64_t Elf64_Xword;
typedef int64_t  Elf64_Sxword;
typedef uint32_t Elf64_Word;
typedef uint16_t Elf64_Half;

#define PT_LOAD    1
#define PT_DYNAMIC 2
#define PT_INTERP  3

#define DT_NULL     0
#define DT_NEEDED   1
#define DT_PLTRELSZ 2
#define DT_PLTGOT   3
#define DT_HASH     4
#define DT_STRTAB   5
#define DT_SYMTAB   6
#define DT_RELA     7
#define DT_RELASZ   8
#define DT_RELAENT  9
#define DT_STRSZ   10
#define DT_SYMENT  11
#define DT_INIT    12
#define DT_FINI    13
#define DT_SONAME  14
#define DT_RPATH   15
#define DT_JMPREL  23
#define DT_PLTREL  20

#define R_X86_64_NONE       0
#define R_X86_64_64         1
#define R_X86_64_GLOB_DAT   6
#define R_X86_64_JUMP_SLOT  7
#define R_X86_64_RELATIVE   8

#define ELF64_R_SYM(i)     ((i) >> 32)
#define ELF64_R_TYPE(i)    ((i) & 0xffffffffL)

/* ELF64 Header */
typedef struct {
    unsigned char e_ident[16];
    Elf64_Half    e_type;
    Elf64_Half    e_machine;
    Elf64_Word    e_version;
    Elf64_Addr    e_entry;
    Elf64_Off     e_phoff;
    Elf64_Off     e_shoff;
    Elf64_Word    e_flags;
    Elf64_Half    e_ehsize;
    Elf64_Half    e_phentsize;
    Elf64_Half    e_phnum;
    Elf64_Half    e_shentsize;
    Elf64_Half    e_shnum;
    Elf64_Half    e_shstrndx;
} Elf64_Ehdr;

/* Program Header */
typedef struct {
    Elf64_Word  p_type;
    Elf64_Word  p_flags;
    Elf64_Off   p_offset;
    Elf64_Addr  p_vaddr;
    Elf64_Addr  p_paddr;
    Elf64_Xword p_filesz;
    Elf64_Xword p_memsz;
    Elf64_Xword p_align;
} Elf64_Phdr;

/* Dynamic Entry */
typedef struct {
    Elf64_Sxword d_tag;
    union {
        Elf64_Xword d_val;
        Elf64_Addr  d_ptr;
    } d_un;
} Elf64_Dyn;

/* Relocation Entry (with addend) */
typedef struct {
    Elf64_Addr    r_offset;
    Elf64_Xword   r_info;
    Elf64_Sxword  r_addend;
} Elf64_Rela;

/* Symbol Table Entry */
typedef struct {
    Elf64_Word    st_name;
    unsigned char st_info;
    unsigned char st_other;
    Elf64_Half    st_shndx;
    Elf64_Addr    st_value;
    Elf64_Xword   st_size;
} Elf64_Sym;

/* ===== Library Search Paths ===== */

static const char *search_paths[] = {
    "/lib",
    "/usr/lib",
    NULL
};

/* ===== Dynamic Linking State ===== */

/* Maximum number of loaded shared libraries */
#define MAX_LIBS 32

typedef struct {
    const char  *name;       /* Library name (from DT_NEEDED) */
    Elf64_Addr   base;       /* Load base address */
    Elf64_Dyn   *dynamic;    /* PT_DYNAMIC segment */
    Elf64_Sym   *symtab;     /* Symbol table */
    const char  *strtab;     /* String table */
    int          loaded;     /* Whether library is loaded */
} LoadedLib;

static LoadedLib loaded_libs[MAX_LIBS];
static int num_loaded_libs = 0;

/* ===== Relocation Processing ===== */

/**
 * Process RELA relocations for a loaded object.
 *
 * @param base     Load base address of the object
 * @param rela     Pointer to the RELA table
 * @param rela_sz  Size of the RELA table in bytes
 * @param symtab   Symbol table
 * @param strtab   String table
 */
static void process_rela(Elf64_Addr base, Elf64_Rela *rela, size_t rela_sz,
                         Elf64_Sym *symtab, const char *strtab)
{
    size_t count = rela_sz / sizeof(Elf64_Rela);

    for (size_t i = 0; i < count; i++) {
        Elf64_Addr *target = (Elf64_Addr *)(base + rela[i].r_offset);
        uint32_t type = ELF64_R_TYPE(rela[i].r_info);
        uint32_t sym_idx = ELF64_R_SYM(rela[i].r_info);

        switch (type) {
        case R_X86_64_RELATIVE:
            /* Base + addend -- no symbol lookup needed */
            *target = base + rela[i].r_addend;
            break;

        case R_X86_64_GLOB_DAT:
        case R_X86_64_JUMP_SLOT:
            /* Symbol value -- look up in symbol table */
            if (symtab && sym_idx > 0) {
                Elf64_Sym *sym = &symtab[sym_idx];
                if (sym->st_value != 0) {
                    *target = base + sym->st_value + rela[i].r_addend;
                }
                /* TODO: cross-library symbol lookup */
            }
            break;

        case R_X86_64_64:
            /* S + A -- symbol value + addend */
            if (symtab && sym_idx > 0) {
                Elf64_Sym *sym = &symtab[sym_idx];
                *target = base + sym->st_value + rela[i].r_addend;
            }
            break;

        case R_X86_64_NONE:
            break;
        }
    }
}

/**
 * Parse the PT_DYNAMIC segment and extract key addresses.
 *
 * @param dynamic  Pointer to the dynamic section
 * @param base     Load base address
 * @param out_sym  Output: symbol table pointer
 * @param out_str  Output: string table pointer
 */
static void parse_dynamic(Elf64_Dyn *dynamic, Elf64_Addr base,
                          Elf64_Sym **out_sym, const char **out_str,
                          Elf64_Rela **out_rela, size_t *out_relasz,
                          Elf64_Rela **out_jmprel, size_t *out_pltrelsz,
                          Elf64_Addr *out_init)
{
    *out_sym = NULL;
    *out_str = NULL;
    *out_rela = NULL;
    *out_relasz = 0;
    *out_jmprel = NULL;
    *out_pltrelsz = 0;
    *out_init = 0;

    for (Elf64_Dyn *d = dynamic; d->d_tag != DT_NULL; d++) {
        switch (d->d_tag) {
        case DT_SYMTAB:
            *out_sym = (Elf64_Sym *)(base + d->d_un.d_ptr);
            break;
        case DT_STRTAB:
            *out_str = (const char *)(base + d->d_un.d_ptr);
            break;
        case DT_RELA:
            *out_rela = (Elf64_Rela *)(base + d->d_un.d_ptr);
            break;
        case DT_RELASZ:
            *out_relasz = d->d_un.d_val;
            break;
        case DT_JMPREL:
            *out_jmprel = (Elf64_Rela *)(base + d->d_un.d_ptr);
            break;
        case DT_PLTRELSZ:
            *out_pltrelsz = d->d_un.d_val;
            break;
        case DT_INIT:
            *out_init = base + d->d_un.d_ptr;
            break;
        }
    }
}

/* ===== dlopen / dlsym / dlclose API (stubs) ===== */

/**
 * Open a shared library by name.
 * Returns a handle (library index) or -1 on failure.
 */
int dlopen(const char *filename, int flags)
{
    (void)flags;
    if (!filename || num_loaded_libs >= MAX_LIBS)
        return -1;

    /* Check if already loaded */
    for (int i = 0; i < num_loaded_libs; i++) {
        if (loaded_libs[i].loaded && loaded_libs[i].name) {
            /* Simple name comparison */
            const char *a = loaded_libs[i].name;
            const char *b = filename;
            int match = 1;
            while (*a && *b) {
                if (*a++ != *b++) { match = 0; break; }
            }
            if (match && !*a && !*b)
                return i; /* Already loaded */
        }
    }

    /* Would load library here -- requires mmap + ELF parsing */
    return -1;
}

/**
 * Look up a symbol in a loaded library.
 */
void *dlsym(int handle, const char *symbol)
{
    if (handle < 0 || handle >= num_loaded_libs)
        return NULL;
    if (!loaded_libs[handle].loaded || !loaded_libs[handle].symtab)
        return NULL;

    /* Linear search through symbol table */
    /* NOTE: Full implementation would use DT_HASH or DT_GNU_HASH */
    (void)symbol;
    return NULL;
}

/**
 * Close a shared library handle.
 */
int dlclose(int handle)
{
    if (handle < 0 || handle >= num_loaded_libs)
        return -1;
    loaded_libs[handle].loaded = 0;
    return 0;
}

/* ===== Entry Point ===== */

/**
 * Dynamic linker entry point.
 *
 * Called by the kernel when loading an ELF with PT_INTERP pointing to
 * /lib/ld-veridian.so. The kernel maps this linker and the program,
 * then transfers control here.
 *
 * argv[0] = program path
 * The auxiliary vector (after envp) contains AT_PHDR, AT_PHNUM,
 * AT_ENTRY, AT_BASE for the loaded program.
 */
void _start(void)
{
    /* The dynamic linker would:
     * 1. Read auxiliary vector to find program's PHDRs
     * 2. Find PT_DYNAMIC in program headers
     * 3. Process DT_NEEDED entries (load shared libraries)
     * 4. Process relocations (RELA, JMPREL)
     * 5. Call DT_INIT functions
     * 6. Transfer to program's e_entry
     *
     * For now, just exit -- full implementation requires the kernel
     * to pass the auxiliary vector and support mmap() for library loading.
     */

    /* Exit via syscall */
    __asm__ volatile(
        "mov $60, %%rax\n"  /* sys_exit */
        "xor %%rdi, %%rdi\n" /* exit code 0 */
        "syscall\n"
        ::: "rax", "rdi"
    );
    __builtin_unreachable();
}
