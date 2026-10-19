# ADR 0017: W^X by default, a per-program opt-in, and Linux's MDWE

- Status: accepted
- Date: 2026-10-17
- Tracking: v0.27.0 syscall coverage (memory group); N-241 follow-up. The user chose a per-program
  opt-in on 2026-10-17 over keeping W^X absolute or adopting Linux's default.

## Context

VeridianOS refuses memory that is writable and executable at once: mmap and mprotect with
PROT_WRITE|PROT_EXEC fail with EACCES. Linux allows both. Programs that write code in place and
run it without remapping need the Linux behaviour: JIT compilers such as LuaJIT, V8 and
JavaScriptCore (QtQml's JIT among them), libffi closures on some targets, and emulators with
dynamic recompilation. Under strict W^X they either break or fall back to an interpreter.

Other systems settle this in one of three ways:

- **OpenBSD:** W^X always. A program opts in with a program header (`PT_OPENBSD_WXNEEDED`, set by
  `ld -z wxneeded`), and only on file systems mounted `wxallowed`.
- **FreeBSD:** a feature-control note (`NT_FREEBSD_FEATURE_CTL`) with a W^X bit, which elfctl(1)
  can flip in place.
- **NetBSD (PaX MPROTECT):** a note set with paxctl(8).
- **Linux 6.3 and later** add the other direction, Memory-Deny-Write-Execute: `PR_SET_MDWE`. It
  refuses W+X and memory gaining exec, is one way, and is inherited across fork and exec unless
  set with `PR_MDWE_NO_INHERIT`. systemd sets it for services with `MemoryDenyWriteExecute=`.

## Decision

1. **W^X stays the default.** mmap of PROT_WRITE|PROT_EXEC and mprotect to it are EACCES. Gaining
   exec (RW to RX) is allowed, as a JIT that remaps needs it.
2. **The opt-in is an ELF note in the main program**, read at exec.
   - Layout: owner `VeridianOS`, type `NT_VERIDIAN_FEATURE_CTL` (1), a 32-bit word of feature
     bits; bit 0 (`VERIDIAN_FCTL_WXNEEDED`) asks for W+X. It sits in any PT_NOTE segment.
   - `<veridian/wxneeded.h>` emits the note: one include in one source file. Linux ignores the
     note, so the binary is unchanged there.
   - Only the main program's note counts. An interpreter or a shared library cannot opt a
     program in, which is OpenBSD's rule.
   - A malformed note segment means no opt-in, and exec does not fail. Refusing would let one
     vendor's bad note break programs, and treating it as no opt-in keeps the safe default.
3. **No run-time switch turns W^X off.** A prctl to opt in would let code injected into any
   process undo the policy (OpenBSD refuses one for the same reason).
4. **PR_SET_MDWE and PR_GET_MDWE follow Linux exactly**, as the stricter direction.
   - The argument checks: EINVAL for unknown bits, for NO_INHERIT alone and for nonzero unused
     arguments.
   - One way: EPERM for any change to bits already set.
   - Under MDWE, W+X is refused even with the note, and so is memory becoming executable that was
     not. A W+X segment in a program exec'd under MDWE fails the exec with EACCES, as Linux's
     mmap of the segment does.
   - Inheritance: kept across fork and exec, both dropped in the new address space under
     NO_INHERIT (`mmf_init_legacy_flags`).
   - PR_GET_MDWE reports MDWE only; the default W^X is not reported as MDWE, because MDWE also
     refuses exec gain, which the default allows.
5. **The policy belongs to the address space** (`mm::wx::WxPolicy` in `VirtualAddressSpace`),
   as Linux keeps MDWE in the mm.
   - A fork child keeps the opt-in (it runs the same program) and inherited MDWE.
   - A CLONE_VM child keeps the policy whole, sharing the mm on Linux.
   - Exec replaces the opt-in with the new program's note.

## Consequences

- Programs built for VeridianOS that need W+X add one include. W^X still covers everything else,
  including Linux binaries that never asked for W+X.
- An unmodified Linux binary that needs W+X does not carry the note and gets EACCES; it has to be
  relinked with the header. A tool that adds the note to an existing binary (elfctl-style) is left
  for later and is listed in docs/KNOWN-LIMITATIONS.md.
- The program's own PT_LOAD segments are mapped as linked, W+X included, unless MDWE is set;
  W^X governs only memory the program maps or changes at run time.
- Tests:
  - host: `mm::wx` (policy, MDWE rules, inheritance) and the note reader, malformed notes
    included (`elf::image`);
  - runtime, on all three architectures: `musl_wx_refused_by_default`, `musl_wx_opt_in`
    (userland/tests/wx_needed_test.c writes code into an RWX mapping and runs it) and
    `musl_mdwe`.

## Update (2026-10-17): the tool

`tools/veridian-elfctl` adds the note to an existing binary, as elfctl(1) and paxctl(8) do for
theirs, so a Linux binary that needs W+X no longer has to be relinked:

- `show FILE`, and `set wxneeded` / `clear wxneeded FILE [-o OUT]`;
- a binary that has the note gets its feature word changed in place;
- otherwise the note is added the way patchelf grows a program header table: a new table (the
  old entries, a PT_LOAD that maps it, the PT_NOTE, and PT_PHDR moved or added) is written at the
  end of the file, so AT_PHDR still names a loaded table for musl's and glibc's loaders.

Patched static, static-PIE and dynamic programs (musl and glibc, x86_64, AArch64 and RISC-V,
64 KiB alignment included) still run on Linux, and `musl_wx_note_tool` checks the kernel's reading
of them on all three architectures (`scripts/build-wx-note-tests.sh`). The decision above is
unchanged.
