#!/usr/bin/env python3
"""Fail on inline assembly that claims `preserves_flags` but sets the flags.

`options(preserves_flags)` promises LLVM that an `asm!` block leaves the
condition flags (AArch64 NZCV, x86 RFLAGS) as they were, so LLVM may keep a
comparison made before the block live across it. A block that runs `cmp`,
`adds`, `test`, `add` (on x86) and the like breaks that promise, and code
after it branches on the block's flags instead. It cost a release-only
AArch64 bug in which passing boot tests reported "failed" (the PL011 print
loop, 2026-10-16).

Usage: scripts/dev/check-asm-flags.py [root]   (default: the repository)
Exits 1 and lists each offending block, 0 if there is none.
"""

import pathlib
import re
import sys

# Mnemonics that write the flags. AArch64: the flag-setting forms only
# (`add`/`sub` do not set NZCV there). x86: arithmetic and logic, shifts and
# rotates, bit tests, compare-and-exchange and flag-register writes. RISC-V
# has no flags register.
AARCH64 = {
    "cmp",
    "cmn",
    "tst",
    "adds",
    "subs",
    "ands",
    "bics",
    "adcs",
    "sbcs",
    "negs",
    "ngcs",
    "ccmp",
    "ccmn",
    "fcmp",
    "fcmpe",
    "fccmp",
    "fccmpe",
    "setf8",
    "setf16",
    "rmif",
    "cfinv",
    "axflag",
    "xaflag",
}
X86 = {
    "add",
    "adc",
    "sub",
    "sbb",
    "cmp",
    "test",
    "and",
    "or",
    "xor",
    "neg",
    "inc",
    "dec",
    "shl",
    "shr",
    "sar",
    "sal",
    "rol",
    "ror",
    "rcl",
    "rcr",
    "shld",
    "shrd",
    "bt",
    "bts",
    "btr",
    "btc",
    "bsf",
    "bsr",
    "tzcnt",
    "lzcnt",
    "popcnt",
    "cmpxchg",
    "cmpxchg8b",
    "cmpxchg16b",
    "xadd",
    "imul",
    "mul",
    "div",
    "idiv",
    "popf",
    "popfq",
    "popfd",
    "sahf",
    "stc",
    "clc",
    "cmc",
    "std",
    "cld",
    "sti",
    "cli",
    "comisd",
    "comiss",
    "ucomisd",
    "ucomiss",
    "ptest",
    "rdrand",
    "rdseed",
    "lock",
}
FLAG_SETTERS = AARCH64 | X86


def asm_blocks(text):
    """Yield (offset, block text) for each `asm!(...)` invocation."""
    for m in re.finditer(r"asm!\s*\(", text):
        i, depth = m.end(), 1
        while depth and i < len(text):
            if text[i] == "(":
                depth += 1
            elif text[i] == ")":
                depth -= 1
            i += 1
        yield m.start(), text[m.start() : i]


def arch_of(path, text, start):
    """The instruction set a block is written for: from the path, else from
    the nearest preceding `target_arch` cfg, else unknown (both sets)."""
    parts = path.parts
    if "aarch64" in parts:
        return AARCH64
    if "x86_64" in parts:
        return X86
    if "riscv" in parts or "riscv64" in parts:
        return set()  # RISC-V has no flags register to clobber
    before = text[max(0, start - 600) : start]
    found = re.findall(r'target_arch\s*=\s*"(\w+)"', before)
    if found:
        return {"aarch64": AARCH64, "x86_64": X86}.get(found[-1], set())
    return FLAG_SETTERS


def offenders(path):
    text = path.read_text(errors="replace")
    for start, block in asm_blocks(text):
        # Only the options clause counts, not a comment mentioning it.
        if not re.search(r"options\s*\([^)]*\bpreserves_flags\b", block):
            continue
        setters = arch_of(path, text, start)
        for template in re.findall(r'"((?:[^"\\]|\\.)*)"', block):
            for stmt in re.split(r"\\n|;", template):
                stmt = stmt.strip()
                stmt = re.sub(r"^[0-9A-Za-z_.]+:\s*", "", stmt)  # labels
                mnemonic = stmt.split()[0].lower() if stmt else ""
                if mnemonic in setters:
                    line = text.count("\n", 0, start) + 1
                    yield line, stmt


def main():
    root = pathlib.Path(
        sys.argv[1]
        if len(sys.argv) > 1
        else pathlib.Path(__file__).resolve().parents[2]
    )
    bad = 0
    for path in sorted((root / "kernel" / "src").rglob("*.rs")):
        for line, stmt in offenders(path):
            print(f"{path.relative_to(root)}:{line}: preserves_flags with `{stmt}`")
            bad += 1
    if bad:
        print(f"{bad} asm block statement(s) set the flags they promise to preserve")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
