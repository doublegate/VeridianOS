/*
 * VeridianOS: which system name uname reports (ADR 0021).
 *
 * uname's sysname is "VeridianOS" unless the process is in Linux mode, in
 * which case it is "Linux" (the release, "7.3.0-veridian", is the same in
 * both). The mode is inherited across fork and exec. A program enters it at
 * exec when it carries the note below (include this header in exactly one
 * of its source files) or a GNU ABI tag naming Linux (glibc programs), and
 * at run time with prctl(PR_VERIDIAN_UNAME, PR_VERIDIAN_UNAME_SET, ...).
 * The `linux-uname` command runs a program in Linux mode.
 *
 * Note layout (as <veridian/wxneeded.h>; the feature bits of every
 * VeridianOS feature-control note are combined): namesz 11, descsz 4,
 * type NT_VERIDIAN_FEATURE_CTL (1), name "VeridianOS", then a 32-bit word
 * of feature bits, of which VERIDIAN_FCTL_LINUX_UNAME (bit 1) asks for
 * Linux mode.
 *
 * Linux ignores the note and returns EINVAL for the prctl option.
 */
#ifndef VERIDIAN_LINUX_UNAME_H
#define VERIDIAN_LINUX_UNAME_H

#define NT_VERIDIAN_FEATURE_CTL   1
#define VERIDIAN_FCTL_LINUX_UNAME 0x2

/* prctl(PR_VERIDIAN_UNAME, subcommand, mode, 0, 0) */
#define PR_VERIDIAN_UNAME        0x56455249 /* 'VERI' */
#define PR_VERIDIAN_UNAME_GET    0          /* returns the mode */
#define PR_VERIDIAN_UNAME_SET    1          /* arg3: the mode */
#define PR_VERIDIAN_UNAME_NATIVE 0          /* sysname "VeridianOS" */
#define PR_VERIDIAN_UNAME_LINUX  1          /* sysname "Linux" */

#ifndef VERIDIAN_LINUX_UNAME_NO_NOTE
__asm__(".pushsection .note.veridian.feature_ctl,\"a\",%note\n"
        ".balign 4\n"
        ".long 11\n"         /* namesz: "VeridianOS" and its NUL */
        ".long 4\n"          /* descsz */
        ".long 1\n"          /* NT_VERIDIAN_FEATURE_CTL */
        ".asciz \"VeridianOS\"\n"
        ".balign 4\n"
        ".long 2\n"          /* VERIDIAN_FCTL_LINUX_UNAME */
        ".popsection\n");
#endif

#endif
