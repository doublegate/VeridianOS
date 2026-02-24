/* selfhost_stdlib.c -- stdlib.h include test */
#include <stdlib.h>
int main(void) {
    void *p = malloc(64);
    if (p) free(p);
    const char msg[] = "STDLIB_PASS\n";
    long r; __asm__ volatile("syscall":"=a"(r):"0"(53L),"D"(1L),"S"(msg),"d"(12L):"rcx","r11","memory");
    return 0;
}
