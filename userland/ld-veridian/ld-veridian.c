/*
 * ld-veridian.so -- VeridianOS Dynamic Linker
 *
 * Minimal ELF dynamic linker / runtime loader for VeridianOS.
 * Handles PT_INTERP-delegated program loading:
 *   1. Kernel loads this linker at a fixed base (0x7F00_0000_0000)
 *   2. Linker reads auxiliary vector from the stack to locate the main ELF
 *   3. Processes DT_NEEDED shared libraries (recursive dlopen)
 *   4. Performs relocations on all loaded objects (RELA, JMPREL)
 *   5. Calls DT_INIT functions
 *   6. Transfers control to the application's entry point (AT_ENTRY)
 *
 * Supported relocation types (x86_64):
 *   R_X86_64_RELATIVE (8)  -- base + addend
 *   R_X86_64_GLOB_DAT (6)  -- symbol value
 *   R_X86_64_JUMP_SLOT (7) -- PLT lazy binding
 *   R_X86_64_64 (1)        -- S + A (symbol + addend)
 *
 * Library search path: /lib, /usr/lib
 *
 * Build: x86_64-veridian-gcc -nostdlib -shared -fPIC -o ld-veridian.so ld-veridian.c
 *
 * IMPORTANT: This file uses ONLY raw syscalls (no libc). It runs before any
 * libc initialization. All helper functions are self-contained.
 */

#include <stdint.h>
#include <stddef.h>

/* ===== Syscall Numbers (from veridian/syscall.h) ===== */

#define SYS_PROCESS_EXIT   11   /* sys_exit(code) */
#define SYS_MEMORY_MAP     20   /* mmap(addr, len, prot, flags, fd, off) */
#define SYS_MEMORY_UNMAP   21   /* munmap(addr, len) */
#define SYS_FILE_OPEN      50   /* open(path, flags, mode) */
#define SYS_FILE_CLOSE     51   /* close(fd) */
#define SYS_FILE_READ      52   /* read(fd, buf, len) */
#define SYS_FILE_WRITE     53   /* write(fd, buf, len) */
#define SYS_FILE_SEEK      54   /* lseek(fd, off, whence) */
#define SYS_FILE_STAT      55   /* fstat(fd, stat) */

/* ===== mmap flags (from veridian/mman.h) ===== */

#define PROT_NONE    0
#define PROT_READ    1
#define PROT_WRITE   2
#define PROT_EXEC    4

#define MAP_SHARED   0x01
#define MAP_PRIVATE  0x02
#define MAP_FIXED    0x10
#define MAP_ANON     0x20

#define MAP_FAILED   ((void *)-1)

/* ===== File open flags (from veridian/fcntl.h) ===== */

#define O_RDONLY     0x0001
#define SEEK_SET     0

/* ===== ELF Structures ===== */

typedef uint64_t Elf64_Addr;
typedef uint64_t Elf64_Off;
typedef uint64_t Elf64_Xword;
typedef int64_t  Elf64_Sxword;
typedef uint32_t Elf64_Word;
typedef uint16_t Elf64_Half;

/* Program header p_type values */
#define PT_NULL    0
#define PT_LOAD    1
#define PT_DYNAMIC 2
#define PT_INTERP  3
#define PT_NOTE    4
#define PT_PHDR    6
#define PT_TLS     7

/* Program header p_flags bits */
#define PF_X  0x1  /* Execute */
#define PF_W  0x2  /* Write */
#define PF_R  0x4  /* Read */

/* Dynamic tag values */
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
#define DT_TEXTREL 22

/* Relocation type values (x86_64) */
#define R_X86_64_NONE       0
#define R_X86_64_64         1
#define R_X86_64_GLOB_DAT   6
#define R_X86_64_JUMP_SLOT  7
#define R_X86_64_RELATIVE   8

/* Symbol binding and type */
#define STB_LOCAL   0
#define STB_GLOBAL  1
#define STB_WEAK    2
#define STT_NOTYPE  0
#define STT_OBJECT  1
#define STT_FUNC    2

#define SHN_UNDEF   0
#define SHN_ABS     0xFFF1

/* ELF relocation info accessors */
#define ELF64_R_SYM(i)     ((i) >> 32)
#define ELF64_R_TYPE(i)    ((i) & 0xffffffffUL)
#define ELF64_ST_BIND(i)   ((unsigned char)(i) >> 4)
#define ELF64_ST_TYPE(i)   ((unsigned char)(i) & 0xf)

/* Auxiliary vector types */
#define AT_NULL    0   /* End of auxv */
#define AT_PHDR    3   /* Program headers address */
#define AT_PHENT   4   /* Program header entry size */
#define AT_PHNUM   5   /* Number of program headers */
#define AT_PAGESZ  6   /* Page size */
#define AT_BASE    7   /* Interpreter base address */
#define AT_ENTRY   9   /* Entry point of main binary */
#define AT_RANDOM  25  /* Address of 16 random bytes */
#define AT_EXECFN  31  /* Filename of program */

/* ELF Magic */
#define ELFMAG0    0x7f
#define ELFMAG1    'E'
#define ELFMAG2    'L'
#define ELFMAG3    'F'
#define ELFCLASS64 2
#define ELFDATA2LSB 1
#define ET_DYN     3
#define ET_EXEC    2
#define EM_X86_64  62

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

/* ===== Raw Syscall Wrappers (x86_64 only -- linker is x86_64-specific) ===== */

static inline long
_syscall0(long nr)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long
_syscall1(long nr, long a1)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long
_syscall2(long nr, long a1, long a2)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long
_syscall3(long nr, long a1, long a2, long a3)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long
_syscall4(long nr, long a1, long a2, long a3, long a4)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3), "r"(r10)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long
_syscall6(long nr, long a1, long a2, long a3, long a4, long a5, long a6)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    register long r8  __asm__("r8")  = a5;
    register long r9  __asm__("r9")  = a6;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

/* ===== Low-Level Helpers (no libc) ===== */

/*
 * These helpers exist because the dynamic linker cannot call libc: it runs
 * before libc is mapped. All string/memory operations must be inlined here.
 */

static inline size_t
_strlen(const char *s)
{
    size_t n = 0;
    while (*s++) n++;
    return n;
}

static inline int
_strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

static inline void *
_memset(void *dst, int c, size_t n)
{
    unsigned char *p = dst;
    while (n--) *p++ = (unsigned char)c;
    return dst;
}

static inline void *
_memcpy(void *dst, const void *src, size_t n)
{
    unsigned char *d = dst;
    const unsigned char *s = src;
    while (n--) *d++ = *s++;
    return dst;
}

/* Write a NUL-terminated string to fd 2 (stderr) for debug output */
static void
_write_str(const char *msg)
{
    size_t len = _strlen(msg);
    _syscall3(SYS_FILE_WRITE, 2, (long)msg, (long)len);
}

/* Write a 64-bit value as hex to fd 2 */
static void
_write_hex(uint64_t v)
{
    char buf[19]; /* "0x" + 16 hex digits + NUL */
    buf[0] = '0'; buf[1] = 'x';
    for (int i = 15; i >= 2; i--) {
        int nibble = v & 0xF;
        buf[i] = (nibble < 10) ? ('0' + nibble) : ('a' + nibble - 10);
        v >>= 4;
    }
    buf[18] = '\0';
    _write_str(buf);
}

static void
_fatal(const char *msg)
{
    _write_str("[ld-veridian] FATAL: ");
    _write_str(msg);
    _write_str("\n");
    _syscall1(SYS_PROCESS_EXIT, 127);
    __builtin_unreachable();
}

/* ===== mmap / munmap wrappers ===== */

static void *
_mmap(void *addr, size_t len, int prot, int flags, int fd, long offset)
{
    long ret = _syscall6(SYS_MEMORY_MAP,
                         (long)addr, (long)len, prot, flags, fd, offset);
    return (void *)ret;
}

static int
_munmap(void *addr, size_t len)
{
    return (int)_syscall2(SYS_MEMORY_UNMAP, (long)addr, (long)len);
}

/* ===== File I/O wrappers ===== */

static int
_open(const char *path, int flags)
{
    return (int)_syscall3(SYS_FILE_OPEN, (long)path, flags, 0);
}

static int
_close(int fd)
{
    return (int)_syscall1(SYS_FILE_CLOSE, fd);
}

static long
_read(int fd, void *buf, size_t len)
{
    return _syscall3(SYS_FILE_READ, fd, (long)buf, (long)len);
}

static long
_pread(int fd, void *buf, size_t len, long off)
{
    /* Seek + read (VeridianOS has SYS_FILE_SEEK = 54) */
    _syscall3(SYS_FILE_SEEK, fd, off, SEEK_SET);
    return _read(fd, buf, len);
}

/* ===== Page size ===== */

#define PAGE_SIZE 4096UL

static inline size_t
page_align_up(size_t v)
{
    return (v + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);
}

static inline size_t
page_align_down(size_t v)
{
    return v & ~(PAGE_SIZE - 1);
}

/* ===== Library Search Paths ===== */

static const char *search_paths[] = {
    "/lib",
    "/usr/lib",
    NULL
};

/* ===== Dynamic Linking State ===== */

#define MAX_LIBS 32

/*
 * LoadedLib -- tracks a mapped shared library (or the main executable).
 *
 * 'base' is the slide (difference between where LOAD segments were mapped
 * versus their p_vaddr values).  For position-independent objects (ET_DYN)
 * the kernel may load them at any address; base accounts for that offset.
 */
typedef struct {
    const char  *name;       /* soname / filename */
    Elf64_Addr   base;       /* load base (slide) */
    Elf64_Dyn   *dynamic;    /* PT_DYNAMIC segment virtual address */
    Elf64_Sym   *symtab;     /* DT_SYMTAB pointer */
    const char  *strtab;     /* DT_STRTAB pointer */
    size_t       symtab_cnt; /* number of entries (estimated from hash/gnu-hash) */
    int          loaded;     /* 1 if this slot is active */
} LoadedLib;

static LoadedLib loaded_libs[MAX_LIBS];
static int num_loaded_libs = 0;

/*
 * The main executable's info is kept in slot 0 and filled in during _start.
 * The dynamic linker itself occupies slot 1 (interp_base).
 */

/* ===== Symbol lookup ===== */

/*
 * Scan the symbol table of a single library for a named symbol.
 * Returns the virtual address of the symbol on match, 0 on failure.
 *
 * We iterate from index 1 (index 0 is always STN_UNDEF) up to symtab_cnt.
 * If symtab_cnt is 0 (not yet computed) we stop at the first STN_UNDEF we
 * encounter after index 0.  This is safe for well-formed ELF files because
 * the dynamic symbol table is always followed by some other section.
 */
static Elf64_Addr
lookup_symbol_in_lib(LoadedLib *lib, const char *name)
{
    if (!lib || !lib->loaded || !lib->symtab || !lib->strtab)
        return 0;

    /* Iterate the dynamic symbol table */
    for (size_t i = 1; ; i++) {
        if (lib->symtab_cnt > 0 && i >= lib->symtab_cnt)
            break;

        Elf64_Sym *sym = &lib->symtab[i];

        /* Stop if we hit an all-zero entry (past the end) */
        if (sym->st_name == 0 && sym->st_value == 0 && sym->st_size == 0) {
            if (lib->symtab_cnt == 0)
                break;
        }

        /* Skip undefined and local symbols */
        if (sym->st_shndx == SHN_UNDEF)
            continue;
        if (ELF64_ST_BIND(sym->st_info) == STB_LOCAL)
            continue;

        const char *sym_name = lib->strtab + sym->st_name;
        if (_strcmp(sym_name, name) == 0) {
            /* Absolute symbols (SHN_ABS) have no slide */
            if (sym->st_shndx == SHN_ABS)
                return sym->st_value;
            return lib->base + sym->st_value;
        }
    }
    return 0;
}

/*
 * Global symbol lookup across all loaded libraries.
 * Searches in load order (main executable first, then libraries).
 */
static Elf64_Addr
lookup_symbol_global(const char *name)
{
    for (int i = 0; i < num_loaded_libs; i++) {
        Elf64_Addr addr = lookup_symbol_in_lib(&loaded_libs[i], name);
        if (addr != 0)
            return addr;
    }
    return 0;
}

/* ===== Relocation Processing ===== */

/*
 * Apply a single RELA table to a loaded object.
 *
 * @param base     Load base (slide) of the object being relocated
 * @param rela     Pointer to the RELA table (virtual address, already slid)
 * @param rela_sz  Size of the RELA table in bytes
 * @param symtab   Symbol table (virtual address, already slid)
 * @param strtab   String table (virtual address, already slid)
 */
static void
process_rela(Elf64_Addr base, Elf64_Rela *rela, size_t rela_sz,
             Elf64_Sym *symtab, const char *strtab)
{
    size_t count = rela_sz / sizeof(Elf64_Rela);

    for (size_t i = 0; i < count; i++) {
        Elf64_Addr  *target  = (Elf64_Addr *)(base + rela[i].r_offset);
        uint32_t     type    = ELF64_R_TYPE(rela[i].r_info);
        uint32_t     sym_idx = ELF64_R_SYM(rela[i].r_info);
        Elf64_Addr   sym_val = 0;

        /* Resolve the referenced symbol (if any) */
        if (sym_idx > 0 && symtab && strtab) {
            Elf64_Sym *sym = &symtab[sym_idx];
            if (sym->st_shndx != SHN_UNDEF) {
                /* Defined in this object */
                sym_val = base + sym->st_value;
            } else {
                /* Look up in the global symbol table */
                const char *sym_name = strtab + sym->st_name;
                sym_val = lookup_symbol_global(sym_name);
                if (sym_val == 0) {
                    /* Weak undefined symbols are allowed to be zero */
                    if (ELF64_ST_BIND(sym->st_info) != STB_WEAK) {
                        _write_str("[ld-veridian] WARNING: undefined symbol: ");
                        _write_str(sym_name);
                        _write_str("\n");
                    }
                }
            }
        }

        switch (type) {
        case R_X86_64_NONE:
            break;

        case R_X86_64_RELATIVE:
            /* B + A: base + addend -- no symbol lookup */
            *target = base + (Elf64_Addr)rela[i].r_addend;
            break;

        case R_X86_64_GLOB_DAT:
            /* S: symbol value (no addend for GLOB_DAT) */
            *target = sym_val;
            break;

        case R_X86_64_JUMP_SLOT:
            /* S: symbol value -- PLT slot; addend is ignored per ABI */
            *target = sym_val;
            break;

        case R_X86_64_64:
            /* S + A: symbol value + addend */
            *target = sym_val + (Elf64_Addr)rela[i].r_addend;
            break;
        }
    }
}

/* ===== PT_DYNAMIC Parsing ===== */

/*
 * Walk a PT_DYNAMIC segment and fill in key addresses.
 *
 * All output pointer parameters are set to 0/NULL first. 'base' is the
 * load slide of the object to which 'dynamic' belongs.
 */
static void
parse_dynamic(Elf64_Dyn *dynamic, Elf64_Addr base,
              Elf64_Sym **out_sym,     const char **out_str,
              Elf64_Rela **out_rela,   size_t *out_relasz,
              Elf64_Rela **out_jmprel, size_t *out_pltrelsz,
              Elf64_Addr *out_init,    Elf64_Addr *out_fini,
              Elf64_Addr *out_strsz)
{
    *out_sym      = NULL;
    *out_str      = NULL;
    *out_rela     = NULL;
    *out_relasz   = 0;
    *out_jmprel   = NULL;
    *out_pltrelsz = 0;
    *out_init     = 0;
    *out_fini     = 0;
    *out_strsz    = 0;

    for (Elf64_Dyn *d = dynamic; d->d_tag != DT_NULL; d++) {
        switch (d->d_tag) {
        case DT_SYMTAB:
            *out_sym = (Elf64_Sym *)(base + d->d_un.d_ptr);
            break;
        case DT_STRTAB:
            *out_str = (const char *)(base + d->d_un.d_ptr);
            break;
        case DT_STRSZ:
            *out_strsz = (Elf64_Addr)d->d_un.d_val;
            break;
        case DT_RELA:
            *out_rela = (Elf64_Rela *)(base + d->d_un.d_ptr);
            break;
        case DT_RELASZ:
            *out_relasz = (size_t)d->d_un.d_val;
            break;
        case DT_JMPREL:
            *out_jmprel = (Elf64_Rela *)(base + d->d_un.d_ptr);
            break;
        case DT_PLTRELSZ:
            *out_pltrelsz = (size_t)d->d_un.d_val;
            break;
        case DT_INIT:
            *out_init = base + d->d_un.d_ptr;
            break;
        case DT_FINI:
            *out_fini = base + d->d_un.d_ptr;
            break;
        }
    }
}

/* ===== ELF LOAD Segment Mapping ===== */

/*
 * Map all PT_LOAD segments of an ELF from an open file descriptor.
 *
 * For ET_DYN objects the base address is chosen by the kernel (we pass
 * addr=NULL, MAP_PRIVATE|MAP_ANON). For ET_EXEC objects the segments must
 * go at their stated p_vaddr (MAP_FIXED).
 *
 * Returns the load slide (mapped_base - lowest_p_vaddr) on success,
 * or (Elf64_Addr)-1 on failure.
 *
 * After return *out_dynamic is the slid virtual address of the PT_DYNAMIC
 * segment (0 if none).
 */
static Elf64_Addr
map_elf_segments(int fd, const Elf64_Ehdr *ehdr,
                 const Elf64_Phdr *phdrs, int is_dyn,
                 Elf64_Addr preferred_base,
                 Elf64_Dyn **out_dynamic)
{
    *out_dynamic = NULL;

    /* Find the extent of all LOAD segments */
    Elf64_Addr vaddr_min = (Elf64_Addr)-1;
    Elf64_Addr vaddr_max = 0;
    int has_load = 0;

    for (int i = 0; i < (int)ehdr->e_phnum; i++) {
        const Elf64_Phdr *ph = &phdrs[i];
        if (ph->p_type != PT_LOAD)
            continue;
        has_load = 1;
        if (ph->p_vaddr < vaddr_min) vaddr_min = ph->p_vaddr;
        if (ph->p_vaddr + ph->p_memsz > vaddr_max)
            vaddr_max = ph->p_vaddr + ph->p_memsz;
    }

    if (!has_load)
        return (Elf64_Addr)-1;

    vaddr_min = page_align_down(vaddr_min);
    vaddr_max = page_align_up(vaddr_max);

    /* For ET_DYN, reserve the whole range with a single anonymous mapping
     * so later MAP_FIXED slices do not race with the kernel choosing
     * addresses. */
    Elf64_Addr slide = 0;

    if (is_dyn) {
        size_t total = vaddr_max - vaddr_min;
        void *reserved = _mmap((void *)preferred_base, total,
                               PROT_NONE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (reserved == MAP_FAILED)
            return (Elf64_Addr)-1;
        slide = (Elf64_Addr)reserved - vaddr_min;
    } else {
        slide = 0; /* ET_EXEC: absolute addresses */
    }

    /* Map each LOAD segment */
    for (int i = 0; i < (int)ehdr->e_phnum; i++) {
        const Elf64_Phdr *ph = &phdrs[i];
        if (ph->p_type != PT_LOAD)
            continue;

        /* Determine protection flags */
        int prot = PROT_NONE;
        if (ph->p_flags & PF_R) prot |= PROT_READ;
        if (ph->p_flags & PF_W) prot |= PROT_WRITE;
        if (ph->p_flags & PF_X) prot |= PROT_EXEC;

        Elf64_Addr seg_vaddr = slide + ph->p_vaddr;
        Elf64_Addr seg_vaddr_aligned = page_align_down(seg_vaddr);
        size_t     file_off_aligned  = page_align_down(ph->p_offset);
        size_t     seg_memsz  = ph->p_memsz  + (seg_vaddr - seg_vaddr_aligned);
        size_t     seg_filesz = ph->p_filesz + (seg_vaddr - seg_vaddr_aligned);

        seg_memsz  = page_align_up(seg_memsz);
        seg_filesz = page_align_up(seg_filesz);

        if (seg_memsz == 0)
            continue;

        /* First map the file-backed portion (PROT_WRITE forced for copy below) */
        if (seg_filesz > 0) {
            void *mapped = _mmap((void *)seg_vaddr_aligned,
                                 seg_filesz,
                                 prot | PROT_WRITE,
                                 MAP_PRIVATE | MAP_FIXED | MAP_ANON,
                                 -1, 0);
            if (mapped == MAP_FAILED)
                return (Elf64_Addr)-1;

            /* Read from file into the anonymous pages */
            long n = _pread(fd, (void *)seg_vaddr_aligned,
                            seg_filesz, (long)file_off_aligned);
            if (n < 0)
                return (Elf64_Addr)-1;
        }

        /* Map additional anonymous (BSS) pages if memsz > filesz */
        if (seg_memsz > seg_filesz) {
            Elf64_Addr bss_start = seg_vaddr_aligned + seg_filesz;
            size_t     bss_size  = seg_memsz - seg_filesz;
            void *mapped = _mmap((void *)bss_start, bss_size,
                                 prot | PROT_WRITE,
                                 MAP_PRIVATE | MAP_FIXED | MAP_ANON,
                                 -1, 0);
            if (mapped == MAP_FAILED)
                return (Elf64_Addr)-1;
            /* Zero the BSS (anonymous pages are already zeroed by the kernel,
             * but zero any partial-page tail explicitly) */
            size_t partial = seg_filesz > 0
                ? (ph->p_filesz % PAGE_SIZE)
                : 0;
            if (partial > 0) {
                _memset((void *)(slide + ph->p_vaddr + ph->p_filesz),
                        0, PAGE_SIZE - partial);
            }
        }

        /* Remove WRITE from read-only segments now that we have copied the data */
        if (!(ph->p_flags & PF_W) && seg_filesz > 0) {
            /* Re-protect without WRITE (best-effort; ignore error) */
            /* NOTE: mprotect is SYS_MEMORY_PROTECT=22 */
            _syscall3(22, (long)seg_vaddr_aligned, (long)seg_filesz, prot);
        }
    }

    /* Locate the PT_DYNAMIC segment (virtual address already slid) */
    for (int i = 0; i < (int)ehdr->e_phnum; i++) {
        const Elf64_Phdr *ph = &phdrs[i];
        if (ph->p_type == PT_DYNAMIC) {
            *out_dynamic = (Elf64_Dyn *)(slide + ph->p_vaddr);
            break;
        }
    }

    return slide;
}

/* ===== Load a Library from the Filesystem ===== */

/*
 * Build the full path "dir/name" into buf (max PATH_MAX chars).
 * Returns 1 on success, 0 if it would overflow.
 */
#define PATH_MAX 512

static int
build_path(char *buf, const char *dir, const char *name)
{
    size_t dlen = _strlen(dir);
    size_t nlen = _strlen(name);
    if (dlen + 1 + nlen + 1 > PATH_MAX)
        return 0;
    _memcpy(buf, dir, dlen);
    buf[dlen] = '/';
    _memcpy(buf + dlen + 1, name, nlen);
    buf[dlen + 1 + nlen] = '\0';
    return 1;
}

/*
 * Read the entire ELF header and program header table from fd.
 *
 * 'phdr_buf' must point to a buffer of at least ehdr->e_phnum * sizeof(Elf64_Phdr).
 * The buffer is allocated via MAP_ANON by the caller.
 */
static int
read_elf_headers(int fd, Elf64_Ehdr *ehdr_out, Elf64_Phdr *phdr_out, size_t phdr_buf_size)
{
    /* Read ELF header */
    if (_pread(fd, ehdr_out, sizeof(Elf64_Ehdr), 0) != sizeof(Elf64_Ehdr))
        return -1;

    /* Validate magic */
    if (ehdr_out->e_ident[0] != ELFMAG0 || ehdr_out->e_ident[1] != ELFMAG1 ||
        ehdr_out->e_ident[2] != ELFMAG2 || ehdr_out->e_ident[3] != ELFMAG3)
        return -1;
    if (ehdr_out->e_ident[4] != ELFCLASS64 || ehdr_out->e_ident[5] != ELFDATA2LSB)
        return -1;
    if (ehdr_out->e_machine != EM_X86_64)
        return -1;

    size_t phdr_size = (size_t)ehdr_out->e_phnum * (size_t)ehdr_out->e_phentsize;
    if (phdr_size > phdr_buf_size)
        return -1;

    if (_pread(fd, phdr_out, phdr_size, (long)ehdr_out->e_phoff) != (long)phdr_size)
        return -1;

    return 0;
}

/* Forward declaration for recursive DT_NEEDED loading */
static void *load_library(const char *name);

/*
 * Process DT_NEEDED entries and load all required libraries.
 * Called after a library's segments are mapped but before relocations.
 */
static void
load_needed_libs(Elf64_Dyn *dynamic, Elf64_Addr base)
{
    if (!dynamic) return;

    /* First pass: find DT_STRTAB */
    const char *strtab = NULL;
    for (Elf64_Dyn *d = dynamic; d->d_tag != DT_NULL; d++) {
        if (d->d_tag == DT_STRTAB) {
            strtab = (const char *)(base + d->d_un.d_ptr);
            break;
        }
    }
    if (!strtab) return;

    /* Second pass: process DT_NEEDED */
    for (Elf64_Dyn *d = dynamic; d->d_tag != DT_NULL; d++) {
        if (d->d_tag == DT_NEEDED) {
            const char *soname = strtab + d->d_un.d_val;
            load_library(soname);
        }
    }
}

/*
 * Register a successfully loaded library into the loaded_libs table.
 * Returns the slot index, or -1 if the table is full.
 */
static int
register_lib(const char *name, Elf64_Addr base, Elf64_Dyn *dynamic,
             Elf64_Sym *symtab, const char *strtab)
{
    if (num_loaded_libs >= MAX_LIBS)
        return -1;
    int slot = num_loaded_libs++;
    loaded_libs[slot].name     = name;  /* points into strtab of the parent */
    loaded_libs[slot].base     = base;
    loaded_libs[slot].dynamic  = dynamic;
    loaded_libs[slot].symtab   = symtab;
    loaded_libs[slot].strtab   = strtab;
    loaded_libs[slot].symtab_cnt = 0;  /* unknown; linear scan will stop on zero */
    loaded_libs[slot].loaded   = 1;
    return slot;
}

/* ===== dlopen Implementation ===== */

/*
 * Load a shared library by filename.
 *
 * Search order: check loaded_libs[] first (avoid duplicates), then search
 * the paths in search_paths[].
 *
 * Returns a pointer to the LoadedLib slot (used as the opaque handle) on
 * success, or NULL on failure.  The dlfcn.h API uses void* handles.
 */
static void *
load_library(const char *name)
{
    if (!name || !*name)
        return NULL;

    /* Check if already loaded (by simple name comparison) */
    for (int i = 0; i < num_loaded_libs; i++) {
        if (loaded_libs[i].loaded && loaded_libs[i].name) {
            if (_strcmp(loaded_libs[i].name, name) == 0)
                return &loaded_libs[i];
        }
    }

    if (num_loaded_libs >= MAX_LIBS)
        return NULL;

    /* Search for the library file */
    char path_buf[PATH_MAX];
    int fd = -1;

    for (int p = 0; search_paths[p]; p++) {
        if (!build_path(path_buf, search_paths[p], name))
            continue;
        fd = _open(path_buf, O_RDONLY);
        if (fd >= 0)
            break;
    }

    if (fd < 0) {
        _write_str("[ld-veridian] cannot find library: ");
        _write_str(name);
        _write_str("\n");
        return NULL;
    }

    /* Allocate a scratch buffer for the ELF + program headers */
    /* We need at most 64 program headers (safe upper bound) */
    size_t phdr_buf_sz = 64 * sizeof(Elf64_Phdr);
    void *phdr_buf = _mmap(NULL, phdr_buf_sz,
                           PROT_READ | PROT_WRITE,
                           MAP_PRIVATE | MAP_ANON, -1, 0);
    if (phdr_buf == MAP_FAILED) {
        _close(fd);
        return NULL;
    }

    Elf64_Ehdr ehdr;
    if (read_elf_headers(fd, &ehdr, (Elf64_Phdr *)phdr_buf, phdr_buf_sz) != 0) {
        _munmap(phdr_buf, phdr_buf_sz);
        _close(fd);
        return NULL;
    }

    int is_dyn = (ehdr.e_type == ET_DYN);

    /* Map all LOAD segments */
    Elf64_Dyn *dynamic = NULL;
    Elf64_Addr slide = map_elf_segments(fd, &ehdr,
                                        (Elf64_Phdr *)phdr_buf,
                                        is_dyn, 0, &dynamic);

    _munmap(phdr_buf, phdr_buf_sz);
    _close(fd);

    if (slide == (Elf64_Addr)-1) {
        _write_str("[ld-veridian] failed to map library: ");
        _write_str(name);
        _write_str("\n");
        return NULL;
    }

    /* Parse the dynamic section */
    Elf64_Sym  *symtab   = NULL;
    const char *strtab   = NULL;
    Elf64_Rela *rela     = NULL;
    size_t      relasz   = 0;
    Elf64_Rela *jmprel   = NULL;
    size_t      pltrelsz = 0;
    Elf64_Addr  init_fn  = 0;
    Elf64_Addr  fini_fn  = 0;
    Elf64_Addr  strsz    = 0;

    if (dynamic) {
        parse_dynamic(dynamic, slide,
                      &symtab, &strtab,
                      &rela, &relasz,
                      &jmprel, &pltrelsz,
                      &init_fn, &fini_fn, &strsz);
    }

    /* Register the library (name pointer into parent strtab -- acceptable
     * because the parent library outlives this one in load order) */
    int slot = register_lib(name, slide, dynamic, symtab, strtab);
    if (slot < 0)
        return NULL;

    /* Recursively load DT_NEEDED dependencies before processing our own
     * relocations, so that all symbols are available */
    load_needed_libs(dynamic, slide);

    /* Suppress unused-variable warnings for fields reserved for future use */
    (void)fini_fn;
    (void)strsz;

    /* Process relocations */
    if (rela && relasz)
        process_rela(slide, rela, relasz, symtab, strtab);
    if (jmprel && pltrelsz)
        process_rela(slide, jmprel, pltrelsz, symtab, strtab);

    /* Call DT_INIT if present */
    if (init_fn) {
        void (*init)(void) = (void (*)(void))init_fn;
        init();
    }

    return &loaded_libs[slot];
}

/* ===== dlopen / dlsym / dlclose (public API) ===== */

/*
 * dlopen -- open a shared library.
 * 'flags' is ignored (we always perform immediate binding).
 * Returns a handle (LoadedLib*) or NULL on failure.
 */
void *
dlopen(const char *filename, int flags)
{
    (void)flags;
    if (!filename)
        return NULL;  /* RTLD_DEFAULT / RTLD_NEXT not supported */
    return load_library(filename);
}

/*
 * dlsym -- look up a symbol in a library.
 * 'handle' must be a value returned by dlopen() (i.e. a LoadedLib *).
 * NULL handle performs a global search across all loaded libraries.
 */
void *
dlsym(void *handle, const char *symbol)
{
    if (!symbol)
        return NULL;

    if (handle == NULL) {
        /* Global search */
        Elf64_Addr addr = lookup_symbol_global(symbol);
        return (addr != 0) ? (void *)addr : NULL;
    }

    LoadedLib *lib = (LoadedLib *)handle;
    Elf64_Addr addr = lookup_symbol_in_lib(lib, symbol);
    return (addr != 0) ? (void *)addr : NULL;
}

/*
 * dlclose -- release a library handle.
 * We do not actually unmap or call DT_FINI: libraries are kept resident
 * for the process lifetime.  This is acceptable for a minimal linker.
 */
int
dlclose(void *handle)
{
    (void)handle;
    return 0;  /* success */
}

/*
 * dlerror -- return last error string.
 * Minimal: always returns NULL (no error tracking).
 */
char *
dlerror(void)
{
    return NULL;
}

/* ===== Entry Point ===== */

/*
 * _start -- Dynamic Linker Entry Point
 *
 * Called by the kernel when it detects PT_INTERP. The kernel has already:
 *   1. Mapped the main ELF's LOAD segments at their stated addresses
 *   2. Mapped this linker at interp_base (0x7F00_0000_0000)
 *   3. Set RSP to point at the standard SysV ABI initial stack:
 *
 *      [RSP]    = argc
 *      [RSP+8]  = argv[0]  (user-space address of program path string)
 *      ...
 *      [RSP+8*(argc+1)] = NULL
 *      [RSP+8*(argc+2)] = envp[0]
 *      ...
 *      envp NULL
 *      auxv pairs (uint64 type, uint64 value)
 *      AT_NULL (0, 0)
 *
 * We receive RSP in RDI because _start is a plain C function and GCC will
 * have touched RSP before our body runs. To avoid this we use naked assembly
 * to capture RSP before the compiler adjusts the stack frame, then call the
 * real C entry.
 *
 * See _start_asm at the bottom of this file.
 */

/*
 * _linker_main -- receives a pointer to the raw initial stack from _start_asm.
 *
 * stack_ptr points to argc on the stack.
 * Returns the application entry point (AT_ENTRY value from auxv).
 */
static Elf64_Addr
_linker_main(uint64_t *stack_ptr)
{
    /* Parse argc / argv / envp / auxv from the initial stack */
    long     argc   = (long)*stack_ptr;
    char   **argv   = (char **)(stack_ptr + 1);
    char   **envp   = argv + argc + 1;  /* skip argv[] + NULL terminator */
    uint64_t *auxv  = (uint64_t *)(envp);

    /* Skip envp to reach the auxiliary vector */
    while (*auxv != 0) auxv++;  /* find the NULL terminating envp */
    auxv++;                     /* step past the NULL */

    /* Walk the auxiliary vector */
    Elf64_Addr at_phdr  = 0;
    Elf64_Addr at_phent = 0;
    Elf64_Addr at_phnum = 0;
    Elf64_Addr at_entry = 0;
    Elf64_Addr at_base  = 0;

    for (uint64_t *av = auxv; av[0] != AT_NULL; av += 2) {
        switch (av[0]) {
        case AT_PHDR:  at_phdr  = av[1]; break;
        case AT_PHENT: at_phent = av[1]; break;
        case AT_PHNUM: at_phnum = av[1]; break;
        case AT_ENTRY: at_entry = av[1]; break;
        case AT_BASE:  at_base  = av[1]; break;
        }
    }

    /*
     * IMPORTANT: AT_PHDR may be 0 if the kernel's prepare_dynamic_linking
     * set phdr_addr = load_base (which may be 0 for a non-PIE executable).
     * In that case we compute phdr_addr from the ELF header at load_base.
     *
     * The main binary's LOAD segments are already in memory (the kernel
     * mapped them before jumping here). We locate the program headers by
     * walking them from AT_PHDR (or from the ELF header if AT_PHDR is 0).
     */

    /* Determine the main binary's load base.
     * For ET_EXEC the ELF header is at its stated virtual address.
     * For ET_DYN  the slide is at_base (but for the MAIN binary, at_base
     * contains the interpreter's base, not the main binary's. The main
     * binary's load base is therefore 0 for non-PIE, or needs to be
     * computed from AT_PHDR for PIE. We use AT_PHDR - phoff for PIE.
     */
    Elf64_Addr main_base = 0;

    if (at_phdr != 0) {
        /* Find the ELF header by scanning backwards from AT_PHDR.
         * The ELF header is at (AT_PHDR - e_phoff) but we do not know e_phoff
         * before we find the header itself.  Walk backwards in 8-byte steps
         * looking for the ELF magic bytes; stop when found or after 512 tries. */
        Elf64_Ehdr *ehdr = (Elf64_Ehdr *)(at_phdr);
        /* Walk backwards to find the ELF magic */
        for (size_t back = 1; back <= 4096; back += 8) {
            Elf64_Ehdr *candidate = (Elf64_Ehdr *)(at_phdr - back * 8);
            if (candidate->e_ident[0] == ELFMAG0 &&
                candidate->e_ident[1] == ELFMAG1 &&
                candidate->e_ident[2] == ELFMAG2 &&
                candidate->e_ident[3] == ELFMAG3) {
                ehdr = candidate;
                break;
            }
        }

        /* main_base = AT_PHDR - ehdr->e_phoff for PIE;
         * for non-PIE (ET_EXEC) e_phoff starts at the file offset, and the
         * ELF is mapped at its absolute vaddr so base = 0. */
        if (ehdr->e_type == ET_DYN) {
            main_base = at_phdr - (Elf64_Addr)ehdr->e_phoff;
        } else {
            main_base = 0;
        }
    }

    /*
     * Register the main executable as loaded_libs[0] so that dlsym and
     * cross-library relocations can search its symbol table.
     */
    if (at_phdr != 0 && at_phnum != 0 && at_phent != 0) {
        Elf64_Phdr *phdrs = (Elf64_Phdr *)at_phdr;
        Elf64_Dyn  *main_dyn = NULL;

        for (Elf64_Addr i = 0; i < at_phnum; i++) {
            Elf64_Phdr *ph = (Elf64_Phdr *)((char *)phdrs + i * at_phent);
            if (ph->p_type == PT_DYNAMIC) {
                main_dyn = (Elf64_Dyn *)(main_base + ph->p_vaddr);
                break;
            }
        }

        if (main_dyn) {
            /* Parse the main binary's dynamic section */
            Elf64_Sym  *main_sym   = NULL;
            const char *main_str   = NULL;
            Elf64_Rela *main_rela  = NULL; size_t main_relasz  = 0;
            Elf64_Rela *main_jmp   = NULL; size_t main_pltsz   = 0;
            Elf64_Addr  main_init  = 0;
            Elf64_Addr  main_fini  = 0;
            Elf64_Addr  main_strsz = 0;

            parse_dynamic(main_dyn, main_base,
                          &main_sym, &main_str,
                          &main_rela, &main_relasz,
                          &main_jmp,  &main_pltsz,
                          &main_init, &main_fini, &main_strsz);

            /* Suppress unused-variable warnings for reserved fields */
            (void)main_fini;
            (void)main_strsz;
            (void)at_base;

            /* Register as slot 0 (main executable) */
            register_lib("<main>", main_base, main_dyn, main_sym, main_str);

            /* Load DT_NEEDED libraries (recursive) */
            load_needed_libs(main_dyn, main_base);

            /* Apply relocations to the main executable */
            if (main_rela && main_relasz)
                process_rela(main_base, main_rela, main_relasz, main_sym, main_str);
            if (main_jmp && main_pltsz)
                process_rela(main_base, main_jmp, main_pltsz, main_sym, main_str);

            /* Call DT_INIT of the main executable */
            if (main_init) {
                void (*init_fn)(void) = (void (*)(void))main_init;
                init_fn();
            }
        }
    }

    /* If AT_ENTRY was not provided, we cannot continue */
    if (at_entry == 0)
        _fatal("AT_ENTRY is 0 -- cannot find program entry point");

    return at_entry;
}

/*
 * _start -- naked assembly stub that captures RSP before any compiler
 * prologue and passes it to _linker_main.
 *
 * On return from _linker_main we receive the program entry point in RAX.
 * We zero all general-purpose registers except RSP (which is already at
 * the correct position, pointing to argc) and jump to the entry point.
 * This matches the SysV ABI initial process state expected by the C runtime.
 *
 * Note: We do NOT use __attribute__((naked)) because GCC's naked function
 * support on x86_64 is unreliable in C (only supported in C++ for some
 * versions). Instead we use a global asm block after the function
 * definitions. The symbol _start is defined in the asm block below.
 *
 * The C-visible _start() function is a placeholder that is never called;
 * the real entry is the asm block.
 */

/*
 * __ld_veridian_entry -- called by the asm _start stub with stack_ptr in RDI.
 * Marked noinline so it always gets a proper call frame (prevents GCC from
 * folding it into the asm stub via tail-call elimination).
 */
static Elf64_Addr __attribute__((noinline))
__ld_veridian_entry(uint64_t *stack_ptr)
{
    return _linker_main(stack_ptr);
}

/* _start is defined by the global asm block below (not as a C function). */

/*
 * The assembly entry point for the dynamic linker.
 *
 * Responsibilities:
 *   1. Capture RSP (which points to argc) into RDI
 *   2. Align the stack to 16 bytes before calling __ld_veridian_entry
 *   3. After __ld_veridian_entry returns RAX = program entry point:
 *      - Restore RSP to the original initial stack (RBX)
 *      - Zero all registers except RSP (SysV ABI process state requirement)
 *      - JMP to the application entry point
 */
__asm__(
    ".global _start\n"
    ".type _start, @function\n"
    "_start:\n"
    "    /* Capture initial RSP (points to argc) */\n"
    "    mov  %rsp, %rdi\n"
    "    mov  %rsp, %rbx\n"         /* save initial RSP across function calls */
    "    /* Align stack to 16 bytes before C call */\n"
    "    and  $-16, %rsp\n"
    "    sub  $8, %rsp\n"           /* 8-byte push for 16-byte alignment at CALL */
    "    /* Call the C entry point (RSP <- argc) */\n"
    "    call __ld_veridian_entry\n"
    "    /* RAX = application entry point (AT_ENTRY) */\n"
    "    /* Restore initial RSP so the C runtime sees a clean stack */\n"
    "    mov  %rbx, %rsp\n"
    "    /* Zero all GP registers (SysV ABI: process start state) */\n"
    "    xor  %rbx, %rbx\n"
    "    xor  %rcx, %rcx\n"
    "    xor  %rdx, %rdx\n"
    "    xor  %rsi, %rsi\n"
    "    xor  %rdi, %rdi\n"
    "    xor  %rbp, %rbp\n"
    "    xor  %r8,  %r8\n"
    "    xor  %r9,  %r9\n"
    "    xor  %r10, %r10\n"
    "    xor  %r11, %r11\n"
    "    xor  %r12, %r12\n"
    "    xor  %r13, %r13\n"
    "    xor  %r14, %r14\n"
    "    xor  %r15, %r15\n"
    "    /* Jump to application entry (RAX) */\n"
    "    jmp  *%rax\n"
    ".size _start, . - _start\n"
);
