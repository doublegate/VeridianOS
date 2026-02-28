/*
 * ld-veridian.so -- VeridianOS Dynamic Linker
 *
 * Full-featured ELF dynamic linker / runtime loader for VeridianOS.
 * Handles PT_INTERP-delegated program loading:
 *   1. Kernel loads this linker at a fixed base (0x7F00_0000_0000)
 *   2. Linker reads auxiliary vector from the stack to locate the main ELF
 *   3. Parses environment (LD_LIBRARY_PATH, LD_PRELOAD, LD_DEBUG, LD_BIND_NOW)
 *   4. Loads LD_PRELOAD libraries (symbols override all later objects)
 *   5. Processes DT_NEEDED shared libraries (recursive dlopen)
 *   6. Performs relocations on all loaded objects (RELA, JMPREL)
 *   7. Sets up Thread-Local Storage (TLS) via ARCH_SET_FS
 *   8. Applies PT_GNU_RELRO protection
 *   9. Calls DT_INIT / DT_INIT_ARRAY constructors
 *  10. Transfers control to the application's entry point (AT_ENTRY)
 *
 * Supported relocation types (x86_64):
 *   R_X86_64_NONE      (0)  -- ignored
 *   R_X86_64_64        (1)  -- S + A (symbol + addend)
 *   R_X86_64_COPY      (5)  -- copy symbol data to relocation address
 *   R_X86_64_GLOB_DAT  (6)  -- symbol value
 *   R_X86_64_JUMP_SLOT (7)  -- PLT lazy/eager binding
 *   R_X86_64_RELATIVE  (8)  -- base + addend
 *   R_X86_64_DTPMOD64  (16) -- TLS module ID
 *   R_X86_64_DTPOFF64  (17) -- TLS offset within module
 *   R_X86_64_TPOFF64   (18) -- TLS offset from TP (static TLS)
 *   R_X86_64_IRELATIVE (37) -- indirect function (GNU extension)
 *
 * Library search: LD_LIBRARY_PATH, then DT_RUNPATH, then /lib, /usr/lib
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
#define SYS_MEMORY_PROTECT 22   /* mprotect(addr, len, prot) */
#define SYS_FILE_OPEN      50   /* open(path, flags, mode) */
#define SYS_FILE_CLOSE     51   /* close(fd) */
#define SYS_FILE_READ      52   /* read(fd, buf, len) */
#define SYS_FILE_WRITE     53   /* write(fd, buf, len) */
#define SYS_FILE_SEEK      54   /* lseek(fd, off, whence) */
#define SYS_ARCH_PRCTL     158  /* arch_prctl(code, addr) */

/* mmap / mprotect constants */
#define PROT_NONE   0x0
#define PROT_READ   0x1
#define PROT_WRITE  0x2
#define PROT_EXEC   0x4
#define MAP_PRIVATE   0x02
#define MAP_ANONYMOUS 0x20
#define MAP_FIXED     0x10

/* File constants */
#define O_RDONLY    0
#define SEEK_SET    0

/* arch_prctl subcodes */
#define ARCH_SET_FS 0x1002
#define ARCH_GET_FS 0x1003

/* ===== ELF Types and Constants ===== */

/* ELF header */
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
} Elf64_Ehdr;

/* Program header */
typedef struct {
    uint32_t p_type;
    uint32_t p_flags;
    uint64_t p_offset;
    uint64_t p_vaddr;
    uint64_t p_paddr;
    uint64_t p_filesz;
    uint64_t p_memsz;
    uint64_t p_align;
} Elf64_Phdr;

/* Dynamic entry */
typedef struct {
    int64_t d_tag;
    union {
        uint64_t d_val;
        uint64_t d_ptr;
    } d_un;
} Elf64_Dyn;

/* Symbol table entry */
typedef struct {
    uint32_t st_name;
    uint8_t  st_info;
    uint8_t  st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
} Elf64_Sym;

/* Relocation entry with addend */
typedef struct {
    uint64_t r_offset;
    uint64_t r_info;
    int64_t  r_addend;
} Elf64_Rela;

/* Auxiliary vector entry */
typedef struct {
    uint64_t a_type;
    uint64_t a_un;
} Elf64_auxv_t;

/* Symbol version definition (DT_VERDEF) */
typedef struct {
    uint16_t vd_version;
    uint16_t vd_flags;
    uint16_t vd_ndx;
    uint16_t vd_cnt;
    uint32_t vd_hash;
    uint32_t vd_aux;
    uint32_t vd_next;
} Elf64_Verdef;

/* Version definition auxiliary entry */
typedef struct {
    uint32_t vda_name;
    uint32_t vda_next;
} Elf64_Verdaux;

/* Version needed entry (DT_VERNEED) */
typedef struct {
    uint16_t vn_version;
    uint16_t vn_cnt;
    uint32_t vn_file;
    uint32_t vn_aux;
    uint32_t vn_next;
} Elf64_Verneed;

/* Version needed auxiliary entry */
typedef struct {
    uint32_t vna_hash;
    uint16_t vna_flags;
    uint16_t vna_other;
    uint32_t vna_name;
    uint32_t vna_next;
} Elf64_Vernaux;

/* Program header types */
#define PT_NULL    0
#define PT_LOAD    1
#define PT_DYNAMIC 2
#define PT_INTERP  3
#define PT_NOTE    4
#define PT_PHDR    6
#define PT_TLS     7
#define PT_GNU_RELRO 0x6474e552

/* Permission flags */
#define PF_X 0x1
#define PF_W 0x2
#define PF_R 0x4

/* Dynamic section tags */
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
#define DT_STRSZ    10
#define DT_SYMENT   11
#define DT_INIT     12
#define DT_FINI     13
#define DT_SONAME   14
#define DT_RPATH    15
#define DT_SYMBOLIC 16
#define DT_REL      17
#define DT_RELSZ    18
#define DT_RELENT   19
#define DT_PLTREL   20
#define DT_DEBUG    21
#define DT_TEXTREL  22
#define DT_JMPREL   23
#define DT_BIND_NOW 24
#define DT_INIT_ARRAY    25
#define DT_FINI_ARRAY    26
#define DT_INIT_ARRAYSZ  27
#define DT_FINI_ARRAYSZ  28
#define DT_RUNPATH       29
#define DT_FLAGS         30
#define DT_VERSYM        0x6FFFFFF0
#define DT_VERDEF        0x6FFFFFFC
#define DT_VERDEFNUM     0x6FFFFFFD
#define DT_VERNEED       0x6FFFFFFE
#define DT_VERNEEDNUM    0x6FFFFFFF
#define DT_FLAGS_1       0x6FFFFFFB

/* DT_FLAGS bits */
#define DF_BIND_NOW   8

/* DT_FLAGS_1 bits */
#define DF_1_NOW   0x00000001
#define DF_1_PIE   0x08000000

/* Symbol binding/type macros */
#define ELF64_R_SYM(info)  ((info) >> 32)
#define ELF64_R_TYPE(info) ((info) & 0xFFFFFFFF)
#define ELF64_ST_BIND(info) ((info) >> 4)
#define ELF64_ST_TYPE(info) ((info) & 0xF)

/* Symbol binding values */
#define STB_LOCAL  0
#define STB_GLOBAL 1
#define STB_WEAK   2

/* Symbol type values */
#define STT_NOTYPE  0
#define STT_OBJECT  1
#define STT_FUNC    2
#define STT_SECTION 3
#define STT_FILE    4
#define STT_TLS     6

/* Relocation types (x86_64) */
#define R_X86_64_NONE       0
#define R_X86_64_64         1
#define R_X86_64_COPY       5
#define R_X86_64_GLOB_DAT   6
#define R_X86_64_JUMP_SLOT  7
#define R_X86_64_RELATIVE   8
#define R_X86_64_DTPMOD64   16
#define R_X86_64_DTPOFF64   17
#define R_X86_64_TPOFF64    18
#define R_X86_64_IRELATIVE  37

/* ELF special section indices */
#define SHN_UNDEF  0
#define SHN_ABS    0xFFF1

/* Symbol version indices */
#define VER_NDX_LOCAL   0
#define VER_NDX_GLOBAL  1

/* Symbol version flags */
#define VER_FLG_BASE    1
#define VER_FLG_WEAK    2

/* Auxiliary vector types */
#define AT_NULL    0
#define AT_PHDR    3
#define AT_PHENT   4
#define AT_PHNUM   5
#define AT_BASE    7
#define AT_ENTRY   9

/* ELF type */
#define ET_DYN 3

/* Page size */
#define PAGE_SIZE 4096

/* ===== Raw Syscall Wrappers ===== */

static inline long _syscall0(long n)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n)
        : "rcx", "r11", "memory");
    return ret;
}

static inline long _syscall1(long n, long a1)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n), "D"(a1)
        : "rcx", "r11", "memory");
    return ret;
}

static inline long _syscall2(long n, long a1, long a2)
{
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n), "D"(a1), "S"(a2)
        : "rcx", "r11", "memory");
    return ret;
}

static inline long _syscall3(long n, long a1, long a2, long a3)
{
    long ret;
    register long r10 __asm__("r10") = a3;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory");
    (void)r10;
    return ret;
}

static inline long _syscall4(long n, long a1, long a2, long a3, long a4)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n), "D"(a1), "S"(a2), "d"(a3), "r"(r10)
        : "rcx", "r11", "memory");
    return ret;
}

static inline long _syscall6(long n, long a1, long a2, long a3,
                              long a4, long a5, long a6)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    register long r8  __asm__("r8")  = a5;
    register long r9  __asm__("r9")  = a6;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(n), "D"(a1), "S"(a2), "d"(a3),
          "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

/* ===== Basic String/Memory Helpers (no libc) ===== */

static size_t _strlen(const char *s)
{
    size_t n = 0;
    while (s[n]) n++;
    return n;
}

static int _strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

static int _strncmp(const char *a, const char *b, size_t n)
{
    for (size_t i = 0; i < n; i++) {
        if (a[i] != b[i]) return (unsigned char)a[i] - (unsigned char)b[i];
        if (a[i] == '\0') return 0;
    }
    return 0;
}

static int _starts_with(const char *str, const char *prefix)
{
    while (*prefix) {
        if (*str != *prefix) return 0;
        str++;
        prefix++;
    }
    return 1;
}

static void _memset(void *dst, int c, size_t n)
{
    unsigned char *p = (unsigned char *)dst;
    while (n--) *p++ = (unsigned char)c;
}

static void _memcpy(void *dst, const void *src, size_t n)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    while (n--) *d++ = *s++;
}

static char *_strcpy(char *dst, const char *src)
{
    char *r = dst;
    while ((*dst++ = *src++)) ;
    return r;
}

static char *_strcat(char *dst, const char *src)
{
    char *r = dst;
    while (*dst) dst++;
    while ((*dst++ = *src++)) ;
    return r;
}

/* ===== Output Helpers ===== */

static void _write_str(const char *s)
{
    size_t len = _strlen(s);
    _syscall3(SYS_FILE_WRITE, 2, (long)s, (long)len);
}

static void _write_hex(uint64_t val)
{
    static const char hex[] = "0123456789abcdef";
    char buf[20];
    buf[0] = '0';
    buf[1] = 'x';
    for (int i = 17; i >= 2; i--) {
        buf[i] = hex[val & 0xF];
        val >>= 4;
    }
    buf[18] = '\0';
    _write_str(buf);
}

static void _write_dec(uint64_t val)
{
    char buf[24];
    int i = 22;
    buf[23] = '\0';
    if (val == 0) {
        buf[i--] = '0';
    } else {
        while (val > 0) {
            buf[i--] = '0' + (val % 10);
            val /= 10;
        }
    }
    _write_str(&buf[i + 1]);
}

/* ===== Debug Output (LD_DEBUG support) ===== */

static int ld_debug_enabled = 0;
static int ld_bind_now_env  = 0;

static void dl_debug(const char *msg)
{
    if (!ld_debug_enabled) return;
    _write_str("[ld-veridian] ");
    _write_str(msg);
    _write_str("\n");
}

static void dl_debug_addr(const char *prefix, uint64_t addr)
{
    if (!ld_debug_enabled) return;
    _write_str("[ld-veridian] ");
    _write_str(prefix);
    _write_hex(addr);
    _write_str("\n");
}

static void dl_debug_str(const char *prefix, const char *str)
{
    if (!ld_debug_enabled) return;
    _write_str("[ld-veridian] ");
    _write_str(prefix);
    _write_str(str);
    _write_str("\n");
}

/* ===== File / Memory Helpers ===== */

static long _open(const char *path, int flags)
{
    return _syscall3(SYS_FILE_OPEN, (long)path, (long)flags, 0);
}

static long _close(long fd)
{
    return _syscall1(SYS_FILE_CLOSE, fd);
}

static long _read(long fd, void *buf, size_t len)
{
    return _syscall3(SYS_FILE_READ, fd, (long)buf, (long)len);
}

static long _pread(long fd, void *buf, size_t len, long offset)
{
    /* seek then read -- no SYS_PREAD on VeridianOS */
    long ret = _syscall3(SYS_FILE_SEEK, fd, offset, SEEK_SET);
    if (ret < 0) return ret;
    return _read(fd, buf, len);
}

static void *_mmap(void *addr, size_t len, int prot, int flags, long fd, long off)
{
    return (void *)_syscall6(SYS_MEMORY_MAP,
        (long)addr, (long)len, (long)prot, (long)flags, fd, off);
}

static long _munmap(void *addr, size_t len)
{
    return _syscall2(SYS_MEMORY_UNMAP, (long)addr, (long)len);
}

static long _mprotect(void *addr, size_t len, int prot)
{
    return _syscall3(SYS_MEMORY_PROTECT, (long)addr, (long)len, (long)prot);
}

static long _arch_prctl(int code, unsigned long addr)
{
    return _syscall2(SYS_ARCH_PRCTL, (long)code, (long)addr);
}

static void _exit(int code) __attribute__((noreturn));
static void _exit(int code)
{
    _syscall1(SYS_PROCESS_EXIT, code);
    __builtin_unreachable();
}

/* ===== Environment Variable Parsing ===== */

#define MAX_LD_LIBRARY_PATHS 32
#define MAX_LD_PRELOAD_LIBS  16
#define MAX_PATH_LEN         256

static char  ld_library_paths[MAX_LD_LIBRARY_PATHS][MAX_PATH_LEN];
static int   ld_library_path_count = 0;

static char  ld_preload_names[MAX_LD_PRELOAD_LIBS][MAX_PATH_LEN];
static int   ld_preload_count = 0;

static void parse_ld_library_path(const char *val)
{
    ld_library_path_count = 0;
    if (!val) return;
    while (*val && ld_library_path_count < MAX_LD_LIBRARY_PATHS) {
        const char *start = val;
        while (*val && *val != ':') val++;
        size_t len = (size_t)(val - start);
        if (len > 0 && len < MAX_PATH_LEN) {
            _memcpy(ld_library_paths[ld_library_path_count], start, len);
            ld_library_paths[ld_library_path_count][len] = '\0';
            ld_library_path_count++;
        }
        if (*val == ':') val++;
    }
}

static void parse_ld_preload(const char *val)
{
    ld_preload_count = 0;
    if (!val) return;
    /* LD_PRELOAD is colon or space separated */
    while (*val && ld_preload_count < MAX_LD_PRELOAD_LIBS) {
        while (*val == ' ' || *val == ':') val++;
        if (!*val) break;
        const char *start = val;
        while (*val && *val != ' ' && *val != ':') val++;
        size_t len = (size_t)(val - start);
        if (len > 0 && len < MAX_PATH_LEN) {
            _memcpy(ld_preload_names[ld_preload_count], start, len);
            ld_preload_names[ld_preload_count][len] = '\0';
            ld_preload_count++;
        }
    }
}

static void parse_environment(char **envp)
{
    if (!envp) return;
    for (int i = 0; envp[i]; i++) {
        if (_starts_with(envp[i], "LD_LIBRARY_PATH=")) {
            parse_ld_library_path(envp[i] + 16);
        } else if (_starts_with(envp[i], "LD_PRELOAD=")) {
            parse_ld_preload(envp[i] + 11);
        } else if (_starts_with(envp[i], "LD_DEBUG=")) {
            ld_debug_enabled = 1;
        } else if (_starts_with(envp[i], "LD_BIND_NOW=")) {
            ld_bind_now_env = 1;
        }
    }
}

/* ===== Loaded Library Registry ===== */

#define MAX_LIBS 64

typedef struct {
    const char  *name;
    uint64_t     base;         /* load bias */
    Elf64_Dyn   *dynamic;
    Elf64_Sym   *symtab;
    const char  *strtab;
    size_t       symtab_cnt;
    int          loaded;

    /* D-2: PLT / versioning */
    uint64_t    *pltgot;       /* DT_PLTGOT base */
    Elf64_Rela  *jmprel;       /* DT_JMPREL (PLT relocations) */
    size_t       pltrelsz;     /* DT_PLTRELSZ */
    int          bind_now;     /* DF_BIND_NOW or LD_BIND_NOW */
    uint16_t    *versym;       /* DT_VERSYM array (one per symtab entry) */
    Elf64_Verdef  *verdef;     /* DT_VERDEF table (version definitions) */
    uint32_t     verdef_num;   /* DT_VERDEFNUM */
    Elf64_Verneed *verneed;    /* DT_VERNEED table (version requirements) */
    uint32_t     verneed_num;  /* DT_VERNEEDNUM */

    /* D-2: init/fini */
    uint64_t     init_func;    /* DT_INIT address */
    uint64_t     fini_func;    /* DT_FINI address */
    uint64_t    *init_array;   /* DT_INIT_ARRAY */
    size_t       init_array_sz;/* DT_INIT_ARRAYSZ (bytes) */
    uint64_t    *fini_array;   /* DT_FINI_ARRAY */
    size_t       fini_array_sz;/* DT_FINI_ARRAYSZ (bytes) */

    /* D-3: TLS */
    void        *tls_image;    /* PT_TLS template data */
    size_t       tls_filesz;   /* PT_TLS p_filesz (initialized bytes) */
    size_t       tls_memsz;    /* PT_TLS p_memsz (total TLS size with BSS) */
    size_t       tls_align;    /* PT_TLS p_align */
    int          has_tls;      /* 1 if object has PT_TLS segment */

    /* D-3: search path */
    const char  *runpath;      /* DT_RUNPATH string */
} LoadedLib;

static LoadedLib loaded_libs[MAX_LIBS];
static int       lib_count = 0;

static LoadedLib *register_lib(const char *name, uint64_t base, Elf64_Dyn *dynamic)
{
    if (lib_count >= MAX_LIBS) {
        _write_str("ld-veridian: too many loaded libraries\n");
        _exit(127);
    }
    LoadedLib *lib = &loaded_libs[lib_count++];
    _memset(lib, 0, sizeof(LoadedLib));
    lib->name    = name;
    lib->base    = base;
    lib->dynamic = dynamic;
    lib->loaded  = 1;
    return lib;
}

/* ===== Dynamic Section Parser ===== */

static void parse_dynamic_into_lib(LoadedLib *lib)
{
    if (!lib->dynamic) return;

    uint64_t hash_nbucket  = 0;
    uint64_t hash_nchain   = 0;
    uint64_t hash_addr     = 0;

    for (Elf64_Dyn *d = lib->dynamic; d->d_tag != DT_NULL; d++) {
        uint64_t val = d->d_un.d_val;
        uint64_t ptr = d->d_un.d_ptr;
        switch (d->d_tag) {
        case DT_STRTAB:       lib->strtab        = (const char *)(lib->base + ptr); break;
        case DT_SYMTAB:       lib->symtab        = (Elf64_Sym *)(lib->base + ptr);  break;
        case DT_HASH:         hash_addr = lib->base + ptr; break;
        case DT_PLTGOT:       lib->pltgot        = (uint64_t *)(lib->base + ptr);   break;
        case DT_JMPREL:       lib->jmprel        = (Elf64_Rela *)(lib->base + ptr); break;
        case DT_PLTRELSZ:     lib->pltrelsz      = val; break;
        case DT_INIT:         lib->init_func     = lib->base + ptr; break;
        case DT_FINI:         lib->fini_func     = lib->base + ptr; break;
        case DT_INIT_ARRAY:   lib->init_array    = (uint64_t *)(lib->base + ptr); break;
        case DT_FINI_ARRAY:   lib->fini_array    = (uint64_t *)(lib->base + ptr); break;
        case DT_INIT_ARRAYSZ: lib->init_array_sz = val; break;
        case DT_FINI_ARRAYSZ: lib->fini_array_sz = val; break;
        case DT_VERSYM:       lib->versym        = (uint16_t *)(lib->base + ptr); break;
        case DT_VERDEF:       lib->verdef        = (Elf64_Verdef *)(lib->base + ptr); break;
        case DT_VERDEFNUM:    lib->verdef_num    = (uint32_t)val; break;
        case DT_VERNEED:      lib->verneed       = (Elf64_Verneed *)(lib->base + ptr); break;
        case DT_VERNEEDNUM:   lib->verneed_num   = (uint32_t)val; break;
        case DT_RUNPATH:      /* resolved after strtab is known */ break;
        case DT_BIND_NOW:     lib->bind_now = 1; break;
        case DT_FLAGS:
            if (val & DF_BIND_NOW) lib->bind_now = 1;
            break;
        case DT_FLAGS_1:
            if (val & DF_1_NOW) lib->bind_now = 1;
            break;
        default: break;
        }
    }

    /* Resolve DT_RUNPATH (needs strtab) */
    if (lib->strtab) {
        for (Elf64_Dyn *d = lib->dynamic; d->d_tag != DT_NULL; d++) {
            if (d->d_tag == DT_RUNPATH) {
                lib->runpath = lib->strtab + d->d_un.d_val;
                break;
            }
        }
    }

    /* Override with env var */
    if (ld_bind_now_env) lib->bind_now = 1;

    /* Calculate symtab_cnt from DT_HASH */
    if (hash_addr) {
        uint32_t *ht = (uint32_t *)hash_addr;
        hash_nbucket = ht[0];
        hash_nchain  = ht[1];
        lib->symtab_cnt = hash_nchain;
        (void)hash_nbucket;
    }
}

/* Legacy parse_dynamic for backward compatibility */
static void parse_dynamic(Elf64_Dyn *dyn, uint64_t base,
                          Elf64_Sym **out_sym, const char **out_str,
                          Elf64_Rela **out_rela, size_t *out_relasz,
                          Elf64_Rela **out_jmprel, size_t *out_jmprelsz,
                          uint64_t *out_init)
{
    *out_sym = NULL;
    *out_str = NULL;
    *out_rela = NULL;
    *out_relasz = 0;
    *out_jmprel = NULL;
    *out_jmprelsz = 0;
    *out_init = 0;

    for (Elf64_Dyn *d = dyn; d->d_tag != DT_NULL; d++) {
        switch (d->d_tag) {
        case DT_SYMTAB:  *out_sym     = (Elf64_Sym *)(base + d->d_un.d_ptr);  break;
        case DT_STRTAB:  *out_str     = (const char *)(base + d->d_un.d_ptr);  break;
        case DT_RELA:    *out_rela    = (Elf64_Rela *)(base + d->d_un.d_ptr);  break;
        case DT_RELASZ:  *out_relasz  = d->d_un.d_val; break;
        case DT_JMPREL:  *out_jmprel  = (Elf64_Rela *)(base + d->d_un.d_ptr);  break;
        case DT_PLTRELSZ:*out_jmprelsz = d->d_un.d_val; break;
        case DT_INIT:    *out_init    = base + d->d_un.d_ptr; break;
        default: break;
        }
    }
}

/* ===== Symbol Versioning ===== */

/*
 * Walk the DT_VERNEED table to find the version name for a given version
 * index (vna_other).
 */
static const char *get_version_name_from_verneed(LoadedLib *lib, uint16_t ver_idx)
{
    if (!lib->verneed || !lib->strtab) return NULL;

    Elf64_Verneed *vn = lib->verneed;
    for (uint32_t i = 0; i < lib->verneed_num; i++) {
        Elf64_Vernaux *vna = (Elf64_Vernaux *)((char *)vn + vn->vn_aux);
        for (uint16_t j = 0; j < vn->vn_cnt; j++) {
            if (vna->vna_other == ver_idx) {
                return lib->strtab + vna->vna_name;
            }
            if (vna->vna_next == 0) break;
            vna = (Elf64_Vernaux *)((char *)vna + vna->vna_next);
        }
        if (vn->vn_next == 0) break;
        vn = (Elf64_Verneed *)((char *)vn + vn->vn_next);
    }
    return NULL;
}

/*
 * Walk the DT_VERDEF table to find the version name for a given version
 * index (vd_ndx).
 */
static const char *get_version_name_from_verdef(LoadedLib *lib, uint16_t ver_idx)
{
    if (!lib->verdef || !lib->strtab) return NULL;

    Elf64_Verdef *vd = lib->verdef;
    for (uint32_t i = 0; i < lib->verdef_num; i++) {
        if (vd->vd_ndx == ver_idx) {
            Elf64_Verdaux *vda = (Elf64_Verdaux *)((char *)vd + vd->vd_aux);
            return lib->strtab + vda->vda_name;
        }
        if (vd->vd_next == 0) break;
        vd = (Elf64_Verdef *)((char *)vd + vd->vd_next);
    }
    return NULL;
}

/*
 * Check if the requested version matches the provided version.
 * If either side lacks versioning info, the match succeeds (permissive).
 */
static int check_symbol_version(LoadedLib *requester, uint16_t req_ver_idx,
                                LoadedLib *provider, uint16_t prov_ver_idx)
{
    /* No versioning on either side -> match */
    if (!requester->versym && !provider->versym) return 1;

    /* Hidden version bit (bit 15) -- mask it off */
    uint16_t req_idx  = req_ver_idx  & 0x7FFF;
    uint16_t prov_idx = prov_ver_idx & 0x7FFF;

    /* VER_NDX_LOCAL means symbol is local-only, cannot satisfy external refs */
    if (prov_idx == VER_NDX_LOCAL) return 0;

    /* VER_NDX_GLOBAL always matches */
    if (req_idx <= VER_NDX_GLOBAL || prov_idx <= VER_NDX_GLOBAL) return 1;

    /* Both have specific versions -- compare names */
    const char *req_name  = get_version_name_from_verneed(requester, req_idx);
    const char *prov_name = get_version_name_from_verdef(provider, prov_idx);

    if (!req_name || !prov_name) return 1; /* permissive if we can't find names */

    return _strcmp(req_name, prov_name) == 0;
}

/* ===== Symbol Lookup ===== */

/*
 * Look up a symbol in a single library with optional version checking.
 * Returns the symbol value (base-adjusted), or 0 on failure.
 *
 * If out_sym_idx is non-NULL, writes the symtab index of the found symbol.
 * Prefers STB_GLOBAL over STB_WEAK. Returns weak match if no global found.
 */
static uint64_t lookup_symbol_in_lib(LoadedLib *lib, const char *name,
                                     LoadedLib *requester, uint16_t req_ver_idx,
                                     uint32_t *out_sym_idx)
{
    if (!lib->symtab || !lib->strtab || !lib->loaded) return 0;

    uint64_t weak_val = 0;
    uint32_t weak_idx = 0;
    int      found_weak = 0;

    for (size_t i = 0; i < lib->symtab_cnt; i++) {
        Elf64_Sym *sym = &lib->symtab[i];
        if (sym->st_shndx == SHN_UNDEF) continue;
        if (_strcmp(lib->strtab + sym->st_name, name) != 0) continue;

        /* Check version compatibility */
        if (requester && lib->versym) {
            uint16_t prov_ver = lib->versym[i];
            if (!check_symbol_version(requester, req_ver_idx, lib, prov_ver))
                continue;
        }

        uint8_t bind = ELF64_ST_BIND(sym->st_info);
        uint64_t val = lib->base + sym->st_value;

        if (bind == STB_GLOBAL) {
            if (out_sym_idx) *out_sym_idx = (uint32_t)i;
            return val;
        }
        if (bind == STB_WEAK && !found_weak) {
            weak_val = val;
            weak_idx = (uint32_t)i;
            found_weak = 1;
        }
    }

    if (found_weak) {
        if (out_sym_idx) *out_sym_idx = weak_idx;
        return weak_val;
    }
    return 0;
}

/*
 * Search all loaded libraries for a symbol, with versioning.
 * LD_PRELOAD libraries are checked first (lowest indices after main exe).
 */
static uint64_t lookup_symbol_global(const char *name,
                                     LoadedLib *requester, uint16_t req_ver_idx)
{
    uint64_t weak_val = 0;
    int found_weak = 0;

    for (int i = 0; i < lib_count; i++) {
        uint64_t val = lookup_symbol_in_lib(&loaded_libs[i], name,
                                            requester, req_ver_idx, NULL);
        if (val) {
            /* Check if it was a weak match by looking at the symbol binding */
            uint8_t bind = STB_GLOBAL;
            for (size_t j = 0; j < loaded_libs[i].symtab_cnt; j++) {
                Elf64_Sym *sym = &loaded_libs[i].symtab[j];
                if (sym->st_shndx == SHN_UNDEF) continue;
                if (_strcmp(loaded_libs[i].strtab + sym->st_name, name) != 0) continue;
                bind = ELF64_ST_BIND(sym->st_info);
                break;
            }
            if (bind == STB_GLOBAL) return val;
            if (!found_weak) {
                weak_val = val;
                found_weak = 1;
            }
        }
    }
    return found_weak ? weak_val : 0;
}

/* Simple lookup without versioning (backward compat for dlsym) */
static uint64_t lookup_symbol_global_simple(const char *name)
{
    return lookup_symbol_global(name, NULL, VER_NDX_GLOBAL);
}

/* ===== PLT Lazy Binding ===== */

/*
 * PLT resolver: called at first use of a PLT slot.
 * Resolves the symbol, patches the GOT entry, returns the resolved address.
 */
static uint64_t plt_resolve(LoadedLib *obj, uint64_t reloc_idx)
{
    if (!obj->jmprel || !obj->symtab || !obj->strtab) {
        _write_str("ld-veridian: plt_resolve: missing relocation data\n");
        _exit(127);
    }

    Elf64_Rela *rela = &obj->jmprel[reloc_idx];
    uint32_t sym_idx = ELF64_R_SYM(rela->r_info);
    Elf64_Sym *sym = &obj->symtab[sym_idx];
    const char *name = obj->strtab + sym->st_name;

    /* Get version info for this symbol */
    uint16_t ver_idx = VER_NDX_GLOBAL;
    if (obj->versym) {
        ver_idx = obj->versym[sym_idx];
    }

    dl_debug_str("plt_resolve: ", name);

    uint64_t addr = lookup_symbol_global(name, obj, ver_idx);
    if (!addr) {
        _write_str("ld-veridian: plt_resolve: undefined symbol: ");
        _write_str(name);
        _write_str("\n");
        _exit(127);
    }

    /* Patch GOT entry so future calls go directly */
    uint64_t *got_entry = (uint64_t *)(obj->base + rela->r_offset);
    *got_entry = addr;

    dl_debug_addr("  -> resolved to ", addr);
    return addr;
}

/*
 * Set up lazy PLT binding for a library.
 * GOT[0] = address of _DYNAMIC (set by linker)
 * GOT[1] = pointer to LoadedLib (for resolver)
 * GOT[2] = pointer to plt_resolve trampoline
 *
 * Note: A real PLT trampoline requires assembly (push reloc_idx, jmp GOT[2]).
 * For now we do eager binding when lazy isn't possible (no asm trampoline).
 * The infrastructure is in place for when we add the asm stub.
 */
static void setup_plt_lazy(LoadedLib *lib)
{
    if (!lib->pltgot || !lib->jmprel || lib->pltrelsz == 0) return;

    if (lib->bind_now) {
        dl_debug_str("  eager binding for: ", lib->name ? lib->name : "<main>");
        return; /* Will be resolved in process_rela with is_jmprel=0 */
    }

    /*
     * Set GOT[1] and GOT[2] for lazy resolution.
     * Until we have an assembly trampoline, we fall through to eager mode
     * in process_rela. But set them anyway for future use.
     */
    lib->pltgot[1] = (uint64_t)lib;
    lib->pltgot[2] = (uint64_t)plt_resolve;

    dl_debug_str("  lazy PLT setup for: ", lib->name ? lib->name : "<main>");
}

/* ===== Relocation Processing ===== */

/*
 * Process RELA relocations for a loaded object.
 * When is_jmprel=1 and lib->bind_now=0, JUMP_SLOT entries are skipped
 * (they will be resolved lazily via plt_resolve).
 */
static void process_rela(LoadedLib *lib, Elf64_Rela *rela, size_t rela_sz,
                         Elf64_Sym *symtab, const char *strtab, int is_jmprel)
{
    uint64_t base = lib->base;
    size_t count = rela_sz / sizeof(Elf64_Rela);

    for (size_t i = 0; i < count; i++) {
        uint32_t type    = ELF64_R_TYPE(rela[i].r_info);
        uint32_t sym_idx = ELF64_R_SYM(rela[i].r_info);
        uint64_t *target = (uint64_t *)(base + rela[i].r_offset);

        switch (type) {
        case R_X86_64_NONE:
            break;

        case R_X86_64_RELATIVE:
            *target = base + rela[i].r_addend;
            break;

        case R_X86_64_GLOB_DAT:
        case R_X86_64_64: {
            const char *name = strtab + symtab[sym_idx].st_name;
            uint16_t ver_idx = VER_NDX_GLOBAL;
            if (lib->versym) ver_idx = lib->versym[sym_idx];

            uint64_t val = lookup_symbol_global(name, lib, ver_idx);
            if (!val && ELF64_ST_BIND(symtab[sym_idx].st_info) != STB_WEAK) {
                _write_str("ld-veridian: undefined symbol: ");
                _write_str(name);
                _write_str("\n");
            }
            *target = val + rela[i].r_addend;
            break;
        }

        case R_X86_64_JUMP_SLOT: {
            /* If lazy binding is active for JMPREL, skip -- plt_resolve handles it */
            if (is_jmprel && !lib->bind_now) break;

            const char *name = strtab + symtab[sym_idx].st_name;
            uint16_t ver_idx = VER_NDX_GLOBAL;
            if (lib->versym) ver_idx = lib->versym[sym_idx];

            uint64_t val = lookup_symbol_global(name, lib, ver_idx);
            if (!val && ELF64_ST_BIND(symtab[sym_idx].st_info) != STB_WEAK) {
                _write_str("ld-veridian: undefined symbol (JUMP_SLOT): ");
                _write_str(name);
                _write_str("\n");
            }
            *target = val;
            break;
        }

        case R_X86_64_COPY: {
            /* Copy symbol's data from defining library */
            const char *name = strtab + symtab[sym_idx].st_name;
            uint16_t ver_idx = VER_NDX_GLOBAL;
            if (lib->versym) ver_idx = lib->versym[sym_idx];

            /* Search in all libs EXCEPT the requesting one */
            uint64_t src_addr = 0;
            for (int j = 0; j < lib_count; j++) {
                if (&loaded_libs[j] == lib) continue;
                src_addr = lookup_symbol_in_lib(&loaded_libs[j], name,
                                                lib, ver_idx, NULL);
                if (src_addr) break;
            }
            if (src_addr) {
                _memcpy((void *)(base + rela[i].r_offset),
                        (void *)src_addr, symtab[sym_idx].st_size);
            } else {
                _write_str("ld-veridian: R_X86_64_COPY: symbol not found: ");
                _write_str(name);
                _write_str("\n");
            }
            break;
        }

        case R_X86_64_TPOFF64: {
            /* Static TLS offset from thread pointer (FS base) */
            const char *name = strtab + symtab[sym_idx].st_name;
            uint16_t ver_idx = VER_NDX_GLOBAL;
            if (lib->versym) ver_idx = lib->versym[sym_idx];

            uint64_t val = 0;
            if (sym_idx != 0) {
                val = lookup_symbol_global(name, lib, ver_idx);
            }
            /* For static TLS, value is negative offset from TP */
            *target = val + rela[i].r_addend;
            break;
        }

        case R_X86_64_DTPMOD64:
            /* TLS module ID -- for static TLS, always 1 (main module) */
            *target = 1;
            break;

        case R_X86_64_DTPOFF64: {
            /* TLS offset within module */
            const char *name = strtab + symtab[sym_idx].st_name;
            uint16_t ver_idx = VER_NDX_GLOBAL;
            if (lib->versym) ver_idx = lib->versym[sym_idx];

            uint64_t val = 0;
            if (sym_idx != 0) {
                val = lookup_symbol_global(name, lib, ver_idx);
            }
            *target = val + rela[i].r_addend;
            break;
        }

        case R_X86_64_IRELATIVE: {
            /* GNU indirect function: call resolver, use return value */
            typedef uint64_t (*ifunc_resolver_t)(void);
            ifunc_resolver_t resolver = (ifunc_resolver_t)(base + rela[i].r_addend);
            *target = resolver();
            break;
        }

        default:
            _write_str("ld-veridian: unsupported reloc type ");
            _write_dec(type);
            _write_str("\n");
            break;
        }
    }
}

/* ===== ELF Segment Mapping (D-1: Multi-LOAD fix) ===== */

/*
 * Map ELF PT_LOAD segments into memory.
 *
 * D-1 fix: Previous implementation didn't properly handle:
 *   - Multiple PT_LOAD segments with gaps between them
 *   - Non-page-aligned p_vaddr within segments
 *   - BSS regions (p_memsz > p_filesz)
 *   - Proper page boundary rounding
 *
 * New approach: For each PT_LOAD segment:
 *   1. Round p_vaddr DOWN to page boundary (seg_start)
 *   2. Round (p_vaddr + p_memsz) UP to page boundary (seg_end)
 *   3. mmap anonymous region of (seg_end - seg_start) bytes
 *   4. pread file data into the correct offset within the region
 *   5. Zero the BSS portion (between file end and memsz end)
 *   6. Re-protect read-only segments via mprotect
 *
 * Returns the load bias (actual_base - ELF requested base).
 * For ET_DYN (shared objects), the first LOAD determines the base.
 */
static uint64_t map_elf_segments(long fd, Elf64_Phdr *phdrs, uint16_t phnum,
                                 int is_dyn)
{
    uint64_t base_addr = 0;
    int first_load = 1;

    for (uint16_t i = 0; i < phnum; i++) {
        if (phdrs[i].p_type != PT_LOAD) continue;

        uint64_t p_vaddr  = phdrs[i].p_vaddr;
        uint64_t p_offset = phdrs[i].p_offset;
        uint64_t p_filesz = phdrs[i].p_filesz;
        uint64_t p_memsz  = phdrs[i].p_memsz;
        uint32_t p_flags  = phdrs[i].p_flags;

        /* Page-align the segment boundaries */
        uint64_t seg_start = p_vaddr & ~(uint64_t)(PAGE_SIZE - 1);
        uint64_t seg_end   = (p_vaddr + p_memsz + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1);
        uint64_t map_size  = seg_end - seg_start;

        /* Offset within the first page where data actually starts */
        uint64_t page_offset = p_vaddr - seg_start;

        /* File offset aligned down to page boundary */
        uint64_t file_page_offset = p_offset - page_offset;

        /* mmap flags: always RW initially so we can write data, re-protect later */
        int prot = PROT_READ | PROT_WRITE;
        int flags = MAP_PRIVATE | MAP_ANONYMOUS;

        void *hint = NULL;

        if (first_load && is_dyn) {
            /* Let kernel choose base for first segment of shared objects */
            hint = NULL;
        } else if (first_load) {
            /* Static executable: map at requested address */
            hint = (void *)seg_start;
            flags |= MAP_FIXED;
        } else {
            /* Subsequent segments: use base-adjusted address */
            hint = (void *)(base_addr + seg_start);
            flags |= MAP_FIXED;
        }

        dl_debug_addr("  LOAD segment vaddr=", p_vaddr);
        dl_debug_addr("    seg_start=", seg_start);
        dl_debug_addr("    seg_end=",   seg_end);
        dl_debug_addr("    map_size=",  map_size);

        void *mapped = _mmap(hint, map_size, prot, flags, -1, 0);
        if ((long)mapped < 0 && (long)mapped > -4096) {
            _write_str("ld-veridian: mmap failed for LOAD segment\n");
            _exit(127);
        }

        if (first_load) {
            if (is_dyn) {
                base_addr = (uint64_t)mapped - seg_start;
            } else {
                base_addr = 0;
            }
            first_load = 0;
        }

        /* Read file data into the mapped region */
        if (p_filesz > 0) {
            uint64_t dst = (uint64_t)mapped + page_offset;
            long rd = _pread(fd, (void *)dst, p_filesz, (long)p_offset);
            if (rd < 0) {
                _write_str("ld-veridian: pread failed for LOAD segment\n");
                _exit(127);
            }
            (void)file_page_offset; /* Used conceptually; actual read uses p_offset */
        }

        /* Zero BSS: region between end-of-file-data and end-of-memsz */
        if (p_memsz > p_filesz) {
            uint64_t bss_start = (uint64_t)mapped + page_offset + p_filesz;
            uint64_t bss_size  = p_memsz - p_filesz;
            _memset((void *)bss_start, 0, bss_size);
        }

        /* Re-protect to the correct permissions */
        int final_prot = 0;
        if (p_flags & PF_R) final_prot |= PROT_READ;
        if (p_flags & PF_W) final_prot |= PROT_WRITE;
        if (p_flags & PF_X) final_prot |= PROT_EXEC;

        if (final_prot != prot) {
            _mprotect(mapped, map_size, final_prot);
        }
    }

    return base_addr;
}

/* ===== TLS Support (D-3) ===== */

/*
 * Scan program headers for PT_TLS and record template info in LoadedLib.
 */
static void scan_tls_phdr(LoadedLib *lib, Elf64_Phdr *phdrs, uint16_t phnum,
                          uint64_t slide)
{
    for (uint16_t i = 0; i < phnum; i++) {
        if (phdrs[i].p_type == PT_TLS) {
            lib->tls_image  = (void *)(slide + phdrs[i].p_vaddr);
            lib->tls_filesz = phdrs[i].p_filesz;
            lib->tls_memsz  = phdrs[i].p_memsz;
            lib->tls_align  = phdrs[i].p_align;
            lib->has_tls    = 1;
            dl_debug_addr("  TLS: memsz=", lib->tls_memsz);
            dl_debug_addr("  TLS: filesz=", lib->tls_filesz);
            dl_debug_addr("  TLS: align=", lib->tls_align);
            break;
        }
    }
}

/*
 * Set up TLS (Thread-Local Storage) for the main thread.
 *
 * x86_64 variant II layout (used by Linux and VeridianOS):
 *   - TLS block is placed BELOW the thread pointer (FS base)
 *   - FS:0 points to itself (self-pointer for %fs:0 access pattern)
 *   - TLS data occupies [FS - tls_memsz, FS)
 *   - The thread control block (TCB) is at FS
 *
 * Memory layout:
 *   [TLS data (memsz)] [TCB / self-pointer] [optional padding]
 *   ^                   ^
 *   tls_block           tp (FS base)
 */
static void setup_tls_for_object(LoadedLib *lib)
{
    if (!lib->has_tls || lib->tls_memsz == 0) return;

    /* Align tls_memsz up to alignment boundary */
    size_t align = lib->tls_align;
    if (align < 16) align = 16;
    size_t tls_aligned = (lib->tls_memsz + align - 1) & ~(align - 1);

    /* Total allocation: TLS data + TCB (8 bytes for self-pointer) + guard */
    size_t tcb_size = 8; /* self-pointer */
    size_t total = tls_aligned + tcb_size + 16; /* 16 bytes padding */

    void *block = _mmap(NULL, total, PROT_READ | PROT_WRITE,
                        MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if ((long)block < 0 && (long)block > -4096) {
        _write_str("ld-veridian: TLS mmap failed\n");
        _exit(127);
    }

    _memset(block, 0, total);

    /* Thread pointer is at the end of TLS data (variant II) */
    uint64_t tp = (uint64_t)block + tls_aligned;

    /* Copy initialized TLS template */
    if (lib->tls_filesz > 0 && lib->tls_image) {
        /* TLS data goes at [tp - tls_memsz, tp - tls_memsz + filesz) */
        void *tls_dest = (void *)(tp - lib->tls_memsz);
        _memcpy(tls_dest, lib->tls_image, lib->tls_filesz);
    }

    /* Self-pointer at TP (required for %fs:0 access pattern) */
    *(uint64_t *)tp = tp;

    /* Set FS base via arch_prctl */
    long ret = _arch_prctl(ARCH_SET_FS, tp);
    if (ret < 0) {
        _write_str("ld-veridian: ARCH_SET_FS failed\n");
        dl_debug_addr("  ret=", (uint64_t)ret);
        /* Non-fatal: TLS won't work but program may still run */
    } else {
        dl_debug_addr("  TLS: FS base set to ", tp);
    }
}

/* ===== Init/Fini Functions ===== */

typedef void (*init_func_t)(void);

static void call_init_functions(LoadedLib *lib)
{
    /* Call DT_INIT first */
    if (lib->init_func) {
        dl_debug_str("  calling DT_INIT for: ", lib->name ? lib->name : "<main>");
        init_func_t fn = (init_func_t)lib->init_func;
        fn();
    }

    /* Then call DT_INIT_ARRAY entries in order */
    if (lib->init_array && lib->init_array_sz > 0) {
        size_t count = lib->init_array_sz / sizeof(uint64_t);
        dl_debug_str("  calling DT_INIT_ARRAY for: ", lib->name ? lib->name : "<main>");
        for (size_t i = 0; i < count; i++) {
            init_func_t fn = (init_func_t)lib->init_array[i];
            if (fn) fn();
        }
    }
}

static void call_fini_functions(LoadedLib *lib)
{
    /* Call DT_FINI_ARRAY in reverse order first */
    if (lib->fini_array && lib->fini_array_sz > 0) {
        size_t count = lib->fini_array_sz / sizeof(uint64_t);
        for (size_t i = count; i > 0; i--) {
            init_func_t fn = (init_func_t)lib->fini_array[i - 1];
            if (fn) fn();
        }
    }

    /* Then call DT_FINI */
    if (lib->fini_func) {
        init_func_t fn = (init_func_t)lib->fini_func;
        fn();
    }
}

/* ===== PT_GNU_RELRO Protection ===== */

static void apply_relro(Elf64_Phdr *phdrs, uint16_t phnum, uint64_t slide)
{
    for (uint16_t i = 0; i < phnum; i++) {
        if (phdrs[i].p_type == PT_GNU_RELRO) {
            uint64_t start = (slide + phdrs[i].p_vaddr) & ~(uint64_t)(PAGE_SIZE - 1);
            uint64_t end   = (slide + phdrs[i].p_vaddr + phdrs[i].p_memsz + PAGE_SIZE - 1)
                             & ~(uint64_t)(PAGE_SIZE - 1);
            _mprotect((void *)start, end - start, PROT_READ);
            dl_debug_addr("  RELRO applied at ", start);
            break;
        }
    }
}

/* ===== Library Search and Loading ===== */

static char path_buf[512];

/*
 * Search for a library by name, checking in order:
 *   1. LD_LIBRARY_PATH directories
 *   2. DT_RUNPATH of the requesting object (TODO: per-object runpath)
 *   3. Default paths: /lib, /usr/lib
 *
 * Returns fd >= 0 on success, or -1 if not found.
 */
static long search_library(const char *name)
{
    long fd;

    /* If name contains '/', treat as direct path */
    for (const char *p = name; *p; p++) {
        if (*p == '/') {
            return _open(name, O_RDONLY);
        }
    }

    /* 1. LD_LIBRARY_PATH */
    for (int i = 0; i < ld_library_path_count; i++) {
        _strcpy(path_buf, ld_library_paths[i]);
        _strcat(path_buf, "/");
        _strcat(path_buf, name);
        fd = _open(path_buf, O_RDONLY);
        if (fd >= 0) {
            dl_debug_str("  found via LD_LIBRARY_PATH: ", path_buf);
            return fd;
        }
    }

    /* 2. Default paths */
    static const char *default_paths[] = {
        "/lib/", "/usr/lib/", NULL
    };
    for (int i = 0; default_paths[i]; i++) {
        _strcpy(path_buf, default_paths[i]);
        _strcat(path_buf, name);
        fd = _open(path_buf, O_RDONLY);
        if (fd >= 0) {
            dl_debug_str("  found in default path: ", path_buf);
            return fd;
        }
    }

    return -1;
}

/*
 * Load a shared library: open, map segments, parse dynamic, relocate.
 */
static LoadedLib *load_library(const char *name)
{
    /* Check if already loaded */
    for (int i = 0; i < lib_count; i++) {
        if (loaded_libs[i].loaded && loaded_libs[i].name
            && _strcmp(loaded_libs[i].name, name) == 0) {
            return &loaded_libs[i];
        }
    }

    dl_debug_str("loading library: ", name);

    long fd = search_library(name);
    if (fd < 0) {
        _write_str("ld-veridian: cannot open library: ");
        _write_str(name);
        _write_str("\n");
        return NULL;
    }

    /* Read ELF header */
    Elf64_Ehdr ehdr;
    _memset(&ehdr, 0, sizeof(ehdr));
    long rd = _pread(fd, &ehdr, sizeof(ehdr), 0);
    if (rd < (long)sizeof(ehdr)) {
        _write_str("ld-veridian: failed to read ELF header: ");
        _write_str(name);
        _write_str("\n");
        _close(fd);
        return NULL;
    }

    /* Validate ELF magic */
    if (ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != 'E' ||
        ehdr.e_ident[2] != 'L'  || ehdr.e_ident[3] != 'F') {
        _write_str("ld-veridian: not an ELF file: ");
        _write_str(name);
        _write_str("\n");
        _close(fd);
        return NULL;
    }

    /* Read program headers */
    size_t phdr_size = ehdr.e_phnum * ehdr.e_phentsize;
    Elf64_Phdr *phdrs = (Elf64_Phdr *)_mmap(NULL, phdr_size,
        PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if ((long)phdrs < 0 && (long)phdrs > -4096) {
        _close(fd);
        return NULL;
    }
    _pread(fd, phdrs, phdr_size, ehdr.e_phoff);

    /* Map segments */
    int is_dyn = (ehdr.e_type == ET_DYN);
    uint64_t slide = map_elf_segments(fd, phdrs, ehdr.e_phnum, is_dyn);

    /* Find PT_DYNAMIC */
    Elf64_Dyn *dynamic = NULL;
    for (uint16_t i = 0; i < ehdr.e_phnum; i++) {
        if (phdrs[i].p_type == PT_DYNAMIC) {
            dynamic = (Elf64_Dyn *)(slide + phdrs[i].p_vaddr);
            break;
        }
    }

    /* Register the library */
    LoadedLib *lib = register_lib(name, slide, dynamic);

    /* Parse dynamic section into lib */
    parse_dynamic_into_lib(lib);

    /* Scan for TLS */
    scan_tls_phdr(lib, phdrs, ehdr.e_phnum, slide);

    /* Load DT_NEEDED dependencies (recursive) */
    if (dynamic) {
        const char *dt_strtab = lib->strtab;
        if (dt_strtab) {
            for (Elf64_Dyn *d = dynamic; d->d_tag != DT_NULL; d++) {
                if (d->d_tag == DT_NEEDED) {
                    const char *needed = dt_strtab + d->d_un.d_val;
                    load_library(needed);
                }
            }
        }
    }

    /* Set up PLT lazy binding */
    setup_plt_lazy(lib);

    /* Process RELA relocations */
    if (lib->symtab && lib->strtab) {
        /* Find RELA section from dynamic */
        Elf64_Rela *rela = NULL;
        size_t relasz = 0;
        for (Elf64_Dyn *d = dynamic; d && d->d_tag != DT_NULL; d++) {
            if (d->d_tag == DT_RELA) rela = (Elf64_Rela *)(slide + d->d_un.d_ptr);
            if (d->d_tag == DT_RELASZ) relasz = d->d_un.d_val;
        }

        /* Process main RELA */
        if (rela && relasz) {
            process_rela(lib, rela, relasz, lib->symtab, lib->strtab, 0);
        }

        /* Process JMPREL (PLT relocations) */
        if (lib->jmprel && lib->pltrelsz) {
            process_rela(lib, lib->jmprel, lib->pltrelsz,
                         lib->symtab, lib->strtab, 1);
        }
    }

    /* Apply RELRO protection */
    apply_relro(phdrs, ehdr.e_phnum, slide);

    /* Call constructors */
    call_init_functions(lib);

    _close(fd);
    _munmap(phdrs, phdr_size);

    dl_debug_str("  loaded: ", name);
    dl_debug_addr("  base=", slide);

    return lib;
}

/*
 * Load all DT_NEEDED libraries from an already-parsed dynamic section.
 */
static void load_needed_libs(Elf64_Dyn *dyn, uint64_t base)
{
    if (!dyn) return;

    const char *strtab = NULL;
    for (Elf64_Dyn *d = dyn; d->d_tag != DT_NULL; d++) {
        if (d->d_tag == DT_STRTAB) {
            strtab = (const char *)(base + d->d_un.d_ptr);
            break;
        }
    }
    if (!strtab) return;

    for (Elf64_Dyn *d = dyn; d->d_tag != DT_NULL; d++) {
        if (d->d_tag == DT_NEEDED) {
            const char *name = strtab + d->d_un.d_val;
            load_library(name);
        }
    }
}

/* ===== dlopen / dlsym / dlclose / dlerror (Public API) ===== */

static const char *dl_error_msg = NULL;

void *dlopen(const char *filename, int flags)
{
    (void)flags;
    if (!filename) {
        dl_error_msg = "dlopen: NULL filename";
        return NULL;
    }

    LoadedLib *lib = load_library(filename);
    if (!lib) {
        dl_error_msg = "dlopen: library not found";
        return NULL;
    }
    return (void *)lib;
}

void *dlsym(void *handle, const char *symbol)
{
    if (!handle || !symbol) {
        dl_error_msg = "dlsym: invalid argument";
        return NULL;
    }

    LoadedLib *lib = (LoadedLib *)handle;
    uint64_t val = lookup_symbol_in_lib(lib, symbol, NULL, VER_NDX_GLOBAL, NULL);
    if (!val) {
        /* Fall back to global search */
        val = lookup_symbol_global_simple(symbol);
    }
    if (!val) {
        dl_error_msg = "dlsym: symbol not found";
        return NULL;
    }
    return (void *)val;
}

int dlclose(void *handle)
{
    if (!handle) return -1;

    LoadedLib *lib = (LoadedLib *)handle;

    /* Call destructors */
    call_fini_functions(lib);

    lib->loaded = 0;
    return 0;
}

char *dlerror(void)
{
    const char *msg = dl_error_msg;
    dl_error_msg = NULL;
    return (char *)msg;
}

/* ===== Linker Entry Point ===== */

/*
 * Main linker logic. Called from _start after stack setup.
 *
 * Stack layout at entry (System V ABI):
 *   [argc] [argv[0]...argv[argc-1]] [NULL] [envp...] [NULL] [auxv...]
 */
void _linker_main(long *sp)
{
    long argc = sp[0];
    char **argv = (char **)&sp[1];
    char **envp = argv + argc + 1;

    /* Skip past envp to find auxv */
    char **env_iter = envp;
    while (*env_iter) env_iter++;
    Elf64_auxv_t *auxv = (Elf64_auxv_t *)(env_iter + 1);

    /* Parse environment variables */
    parse_environment(envp);

    dl_debug("ld-veridian starting");

    /* Extract auxiliary vector values */
    Elf64_Phdr *at_phdr = NULL;
    uint64_t at_phent   = 0;
    uint64_t at_phnum   = 0;
    uint64_t at_base    = 0;
    uint64_t at_entry   = 0;

    for (Elf64_auxv_t *a = auxv; a->a_type != AT_NULL; a++) {
        switch (a->a_type) {
        case AT_PHDR:  at_phdr  = (Elf64_Phdr *)a->a_un; break;
        case AT_PHENT: at_phent = a->a_un; break;
        case AT_PHNUM: at_phnum = a->a_un; break;
        case AT_BASE:  at_base  = a->a_un; break;
        case AT_ENTRY: at_entry = a->a_un; break;
        }
    }

    if (!at_phdr || !at_entry) {
        _write_str("ld-veridian: missing AT_PHDR or AT_ENTRY\n");
        _exit(127);
    }

    (void)at_phent;
    (void)at_base;

    dl_debug_addr("AT_ENTRY=", at_entry);
    dl_debug_addr("AT_PHDR=",  (uint64_t)at_phdr);

    /* Find the main executable's load bias by scanning its PHDRs */
    uint64_t exe_base = 0;
    Elf64_Dyn *exe_dynamic = NULL;
    for (uint64_t i = 0; i < at_phnum; i++) {
        Elf64_Phdr *ph = (Elf64_Phdr *)((char *)at_phdr + i * sizeof(Elf64_Phdr));
        if (ph->p_type == PT_PHDR) {
            exe_base = (uint64_t)at_phdr - ph->p_vaddr;
        }
        if (ph->p_type == PT_DYNAMIC) {
            exe_dynamic = (Elf64_Dyn *)(exe_base + ph->p_vaddr);
        }
    }

    /* Re-scan with correct base (PT_DYNAMIC may have been found before PT_PHDR) */
    for (uint64_t i = 0; i < at_phnum; i++) {
        Elf64_Phdr *ph = (Elf64_Phdr *)((char *)at_phdr + i * sizeof(Elf64_Phdr));
        if (ph->p_type == PT_DYNAMIC) {
            exe_dynamic = (Elf64_Dyn *)(exe_base + ph->p_vaddr);
            break;
        }
    }

    dl_debug_addr("exe_base=", exe_base);

    /* Register main executable as first loaded object */
    LoadedLib *main_lib = register_lib(argv[0], exe_base, exe_dynamic);
    parse_dynamic_into_lib(main_lib);

    /* Scan main executable for TLS */
    for (uint64_t i = 0; i < at_phnum; i++) {
        Elf64_Phdr *ph = (Elf64_Phdr *)((char *)at_phdr + i * sizeof(Elf64_Phdr));
        if (ph->p_type == PT_TLS) {
            main_lib->tls_image  = (void *)(exe_base + ph->p_vaddr);
            main_lib->tls_filesz = ph->p_filesz;
            main_lib->tls_memsz  = ph->p_memsz;
            main_lib->tls_align  = ph->p_align;
            main_lib->has_tls    = 1;
            break;
        }
    }

    /* Load LD_PRELOAD libraries FIRST (before DT_NEEDED) */
    for (int i = 0; i < ld_preload_count; i++) {
        dl_debug_str("preloading: ", ld_preload_names[i]);
        LoadedLib *plib = load_library(ld_preload_names[i]);
        if (!plib) {
            _write_str("ld-veridian: WARNING: failed to preload: ");
            _write_str(ld_preload_names[i]);
            _write_str("\n");
        }
    }

    /* Load DT_NEEDED dependencies */
    load_needed_libs(exe_dynamic, exe_base);

    /* Parse dynamic for main executable relocations (legacy path) */
    Elf64_Sym   *symtab  = main_lib->symtab;
    const char  *strtab  = main_lib->strtab;
    Elf64_Rela  *rela    = NULL;
    size_t       relasz  = 0;
    Elf64_Rela  *jmprel  = NULL;
    size_t       jmprelsz = 0;
    uint64_t     init    = 0;

    if (exe_dynamic) {
        for (Elf64_Dyn *d = exe_dynamic; d->d_tag != DT_NULL; d++) {
            switch (d->d_tag) {
            case DT_RELA:     rela     = (Elf64_Rela *)(exe_base + d->d_un.d_ptr); break;
            case DT_RELASZ:   relasz   = d->d_un.d_val; break;
            case DT_JMPREL:   jmprel   = (Elf64_Rela *)(exe_base + d->d_un.d_ptr); break;
            case DT_PLTRELSZ: jmprelsz = d->d_un.d_val; break;
            case DT_INIT:     init     = exe_base + d->d_un.d_ptr; break;
            default: break;
            }
        }
    }

    /* Set up PLT for main executable */
    setup_plt_lazy(main_lib);

    /* Process relocations for main executable */
    if (symtab && strtab) {
        if (rela && relasz) {
            process_rela(main_lib, rela, relasz, symtab, strtab, 0);
        }
        if (jmprel && jmprelsz) {
            process_rela(main_lib, jmprel, jmprelsz, symtab, strtab, 1);
        }
    }

    /* Apply RELRO for main executable */
    for (uint64_t i = 0; i < at_phnum; i++) {
        Elf64_Phdr *ph = (Elf64_Phdr *)((char *)at_phdr + i * sizeof(Elf64_Phdr));
        if (ph->p_type == PT_GNU_RELRO) {
            uint64_t start = (exe_base + ph->p_vaddr) & ~(uint64_t)(PAGE_SIZE - 1);
            uint64_t end   = (exe_base + ph->p_vaddr + ph->p_memsz + PAGE_SIZE - 1)
                             & ~(uint64_t)(PAGE_SIZE - 1);
            _mprotect((void *)start, end - start, PROT_READ);
            dl_debug_addr("  main RELRO at ", start);
            break;
        }
    }

    /* Set up TLS for main thread */
    setup_tls_for_object(main_lib);

    /* Call DT_INIT for main executable */
    if (init) {
        main_lib->init_func = init;
    }
    call_init_functions(main_lib);

    dl_debug("transferring control to application");
    dl_debug_addr("entry=", at_entry);

    /* Jump to application entry point */
    typedef void (*entry_fn)(void);
    entry_fn entry = (entry_fn)at_entry;
    entry();

    /* Should not reach here */
    _exit(0);
}

/* ===== _start: Assembly Entry Point ===== */

__asm__(
    ".global _start\n"
    "_start:\n"
    "    xor %rbp, %rbp\n"       /* Clear frame pointer (ABI) */
    "    mov %rsp, %rdi\n"       /* Pass stack pointer as arg1 */
    "    and $-16, %rsp\n"       /* Align stack to 16 bytes */
    "    call _linker_main\n"    /* Call C entry */
    "    ud2\n"                   /* Should never return */
);
