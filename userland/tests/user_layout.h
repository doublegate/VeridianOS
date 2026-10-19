/* Shared by the runtime suites: the user address-space limit
 * (mm::user_layout), probed from user space. One copy, included by
 * audit_runtime_test.c and musl_runtime_test.c (it was duplicated in
 * both; PR #20 review). Needs <sys/mman.h>, <stdio.h> and <string.h>. */
#ifndef VERIDIAN_TESTS_USER_LAYOUT_H
#define VERIDIAN_TESTS_USER_LAYOUT_H

/* The first address past user space: 47-bit everywhere but RISC-V under
 * Sv39, where it is 256 GiB less a page (ADR 0018). Found by asking for the
 * last page below the 47-bit limit as a hint: honoured if that is user
 * space, else the kernel picks somewhere else. Elsewhere an ignored hint
 * gives 0, which fails the checks below. */
static unsigned long user_space_end(void)
{
    static unsigned long end;
    static int probed;
    if (!probed) {
        probed = 1;
        void *top = (void *)0x7FFFFFFFE000UL;
        void *p = mmap(top, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        end = p == top ? 0x7FFFFFFFF000UL : 0;
#ifdef __riscv
        if (p != top)
            end = 0x3FFFFFF000UL;
#endif
        if (p != MAP_FAILED)
            munmap(p, 4096);
    }
    return end;
}

#ifdef __riscv
/* The paging mode /proc/cpuinfo reports, as Linux does ("mmu : sv39"):
 * 39 or 48, 0 if it says neither. */
static int riscv_mmu_bits(void)
{
    FILE *f = fopen("/proc/cpuinfo", "r");
    char line[128];
    int bits = 0;
    if (!f)
        return 0;
    while (fgets(line, sizeof(line), f))
        if (strncmp(line, "mmu", 3) == 0)
            bits = strstr(line, "sv39") ? 39 : strstr(line, "sv48") ? 48 : 0;
    fclose(f);
    return bits;
}
#endif

/* Whether the probed limit is the paging mode's: a Sv48 hart that wrongly
 * refused the high page would otherwise pass as Sv39. */
static int user_end_matches_mode(void)
{
    unsigned long end = user_space_end();
#ifdef __riscv
    int bits = riscv_mmu_bits();
    return (bits == 39 && end == 0x3FFFFFF000UL) || (bits == 48 && end == 0x7FFFFFFFF000UL);
#else
    return end == 0x7FFFFFFFF000UL;
#endif
}

#endif
