/*
 * Print the system name uname reports, for musl_uname_linux_mode (ADR 0021),
 * or, given a program, run that instead (to show what an exec from this
 * image sees). scripts/build-uname-programs.sh builds it several ways:
 *
 *   uname_plain_test     no note: inherits the caller's mode
 *   uname_linux_test     -DUNAME_LINUX_NOTE: <veridian/linux-uname.h>
 *   uname_gnu_tag_test   -DUNAME_GNU_TAG: a GNU ABI tag naming Linux, as
 *                        glibc's crt1.o emits for programs built for Linux
 *   uname_patched_test   uname_plain_test with the note added by
 *                        tools/veridian-elfctl
 */
#ifdef UNAME_LINUX_NOTE
#include <veridian/linux-uname.h>
#endif

#include <stdio.h>
#include <sys/utsname.h>
#include <unistd.h>

#ifdef UNAME_GNU_TAG
/* NT_GNU_ABI_TAG (1), owner "GNU": OS 0 (Linux), kernel 3.2.0. */
__asm__(".pushsection .note.ABI-tag,\"a\",%note\n"
        ".balign 4\n"
        ".long 4\n"          /* namesz: "GNU" and its NUL */
        ".long 16\n"         /* descsz */
        ".long 1\n"          /* NT_GNU_ABI_TAG */
        ".asciz \"GNU\"\n"
        ".long 0, 3, 2, 0\n" /* ELF_NOTE_OS_LINUX, 3.2.0 */
        ".popsection\n");
#endif

int main(int argc, char **argv)
{
    struct utsname u;
    if (argc > 1) {
        execv(argv[1], argv + 1);
        return 127;
    }
    if (uname(&u) != 0)
        return 1;
    puts(u.sysname);
    return 0;
}
