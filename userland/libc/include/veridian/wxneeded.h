/*
 * VeridianOS W^X opt-in (ADR 0017).
 *
 * VeridianOS refuses memory that is writable and executable at once
 * (mmap and mprotect with PROT_WRITE|PROT_EXEC fail with EACCES). A program
 * that needs it -- a JIT that writes code in place -- opts in by carrying
 * this note: include this header in exactly one of its source files. The
 * note is read from the main program only, at exec; an interpreter or a
 * library cannot opt in, and nothing can opt in at run time.
 *
 * Layout (an ELF note, in a PT_NOTE segment): namesz 11, descsz 4,
 * type NT_VERIDIAN_FEATURE_CTL (1), name "VeridianOS", then a 32-bit word
 * of feature bits, of which VERIDIAN_FCTL_WXNEEDED (bit 0) is the opt-in.
 *
 * Linux ignores the note, so the same binary runs there unchanged.
 */
#ifndef VERIDIAN_WXNEEDED_H
#define VERIDIAN_WXNEEDED_H

#define NT_VERIDIAN_FEATURE_CTL 1
#define VERIDIAN_FCTL_WXNEEDED  0x1

__asm__(".pushsection .note.veridian.feature_ctl,\"a\",%note\n"
        ".balign 4\n"
        ".long 11\n"         /* namesz: "VeridianOS" and its NUL */
        ".long 4\n"          /* descsz */
        ".long 1\n"          /* NT_VERIDIAN_FEATURE_CTL */
        ".asciz \"VeridianOS\"\n"
        ".balign 4\n"
        ".long 1\n"          /* VERIDIAN_FCTL_WXNEEDED */
        ".popsection\n");

#endif
