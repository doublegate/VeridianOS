# ADR 0010: Dynamic linking with musl's loader

- Status: accepted
- Date: 2026-10-08
- Tracking: X3 of the compatibility plan (loader, auxv), pulled forward from v0.29 into v0.27;
  the part of the v0.29 page cache that shared libraries need.

## Context

The KDE stack (Qt, KDE Frameworks, KWin, Plasma) was being cross-built as static archives
linked into static programs. That design stalls on plugins. KDE and Qt load most of their
functionality at run time: platform and image-format plugins, window decorations, KIO workers,
KCMs, and hundreds of QML modules. With static linking, each program would have to link in and
register every plugin it might ever load, including plugins from packages built after it. On
2026-10-08 the project switched KDE to shared libraries loaded by musl's dynamic linker, as
upstream intends. That needs the kernel half of dynamic linking now, not in v0.29.

The ELF loader had the scaffolding of `PT_INTERP` support but could not run a dynamic program:

- **Images loaded at their link addresses.** Segments were mapped at `p_vaddr`, so a PIE program
  and `ld-musl` (both linked at 0) would land on the null page and on each other.
- **No validation of the program headers.** `p_offset + p_filesz` was sliced out of the file
  unchecked, so a malformed file panicked the kernel. Overflowing `p_vaddr + p_memsz`, segments
  outside user space and `p_filesz > p_memsz` were all accepted.
- **No auxiliary vector for static programs**, which Linux always provides. A dynamic program
  got a vector with AT_PHNUM counted from `PT_PHDR` entries (1, not `e_phnum`), an unrelocated
  AT_BASE (so it was dropped) and no AT_RANDOM, AT_HWCAP, AT_UID or AT_SECURE.
- **Every mapped file page was a private copy.** `mmap` copied file contents into fresh frames,
  and so did the loader. With shared libraries, every process would hold its own copy of Qt and
  of every framework.

What the two sides expect (Linux `fs/binfmt_elf.c`, musl `ldso/dynlink.c`):

- Linux places a PIE program at `ELF_ET_DYN_BASE` (on x86_64 `0x5555_5555_4000`, plus a random
  offset with ASLR) and the interpreter in the mmap area. The interpreter's load bias is AT_BASE,
  and execution starts at its biased entry point.
- The vector: AT_SYSINFO_EHDR and AT_MINSIGSTKSZ (x86 ARCH_DLINFO), then AT_HWCAP, AT_PAGESZ,
  AT_CLKTCK, AT_PHDR, AT_PHENT, AT_PHNUM, AT_BASE, AT_FLAGS, AT_ENTRY, AT_UID, AT_EUID, AT_GID,
  AT_EGID, AT_SECURE, AT_RANDOM (16 bytes on the stack), AT_HWCAP2, AT_EXECFN, AT_PLATFORM
  ("x86_64", on the stack), AT_NULL.
- musl's loader maps a library by reserving its whole span with one private file mapping (which
  may reach past the end of the file), maps each further segment over it with
  `MAP_PRIVATE|MAP_FIXED` at the segment's page-aligned file offset, zeroes the partial page
  after `p_filesz` and maps the rest of the BSS anonymously with `MAP_FIXED`, and after
  relocation makes the RELRO pages read-only with `mprotect`. musl's start code reads AT_RANDOM
  (stack protector), AT_SECURE, AT_PHDR/PHNUM (static TLS) and AT_HWCAP.

## Decision

- **One image loader (`elf::image`).** A pure planning step validates the program headers and
  produces a page plan; the mapping step only follows the plan.
  - Validation: PT_LOAD file ranges inside the file, `p_filesz <= p_memsz`, no address overflow,
    `p_vaddr` and `p_offset` congruent modulo the page size, the biased image inside user space.
    A violation fails the exec with ENOEXEC; the kernel never panics on file contents.
  - Pages shared by two segments get the union of their permissions and both contents.
  - Runs of pages with equal permissions become one mapping each.
- **Load addresses (no randomisation yet).**
  - An ET_EXEC program at its link addresses.
  - An ET_DYN program (PIE, or the loader run directly) at `0x5555_5555_4000`, Linux's
    `ELF_ET_DYN_BASE` without randomisation.
  - The interpreter at an address taken from the mmap area, like any other mapping.
  - The heap keeps its fixed base (`0x2000_0000_0000`), so brk is unaffected.
- **Auxiliary vector for every program,** in Linux's order with Linux's values. AT_SYSINFO_EHDR
  is omitted until there is a vDSO; musl then makes real system calls. AT_MINSIGSTKSZ is the
  largest signal frame the kernel builds.
- **Shared file pages.** A regular file has a page cache (`mm::page_cache`; BlockFS keeps one per
  inode, since it builds a new node object on every lookup): a frame per page that has been
  mapped, holding one owner of the frame (`mm::frame_refs`). A page missing from the cache is
  filled without the cache's lock held, since filling it reads the file.
  - A private mapping of the file takes one more owner per page and maps the frame read-only.
    If the mapping is writable, the pages are COW, so the first write copies the page (ADR 0005),
    and `mprotect` adding write access does the same.
  - The loader maps program and interpreter segments the same way, so every process running a
    program shares its text and read-only data.
  - Writing or truncating the file, or freeing or reusing its inode, drops the cache, so later
    mappings see the new contents; existing private mappings keep the pages they have. That is allowed for
    `MAP_PRIVATE` (POSIX leaves it unspecified), and it is what shared libraries need. Writable
    `MAP_SHARED` mappings of regular files, writeback and cache eviction under memory pressure
    remain v0.29 work.
- **Toolchain and packages.** musl is built shared (`/lib/ld-musl-x86_64.so.1`, `libc.so`), GCC
  with shared `libstdc++` and `libgcc_s`, and the KDE pipeline with shared libraries. The static
  workarounds of the pipeline go away.

## Consequences

- KDE programs load their plugins and QML modules with `dlopen`, as upstream builds them; no
  per-program plugin registration and no cross-package static plugin export is needed.
- Libraries and program text are in memory once, however many processes use them.
- A malformed executable fails the exec instead of panicking the kernel (regression tests).
- **Not yet done:** address-space randomisation of the PIE base, the interpreter, mmap and the
  stack (sprint G, hardening); a vDSO; eviction of cached pages under memory pressure (v0.29).
  Until eviction exists, a file's cache lives as long as its node.
- The native libc and its test programs stay static; musl programs may be either.

## References

- [Linux `fs/binfmt_elf.c`](https://github.com/torvalds/linux/blob/master/fs/binfmt_elf.c):
  `load_elf_binary`, `load_elf_interp`, `create_elf_tables`
- [musl `ldso/dynlink.c`](https://git.musl-libc.org/cgit/musl/tree/ldso/dynlink.c): `map_library`,
  `reloc_all`, `__dls3`
- [getauxval(3)](https://man7.org/linux/man-pages/man3/getauxval.3.html)
- [System V ABI, AMD64 supplement: initial process stack](https://refspecs.linuxbase.org/elf/x86_64-abi-0.98.pdf)
