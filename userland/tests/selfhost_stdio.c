/* selfhost_stdio.c -- stdio.h include test */
#include <stdio.h>
int main(void) {
    const char msg[] = "STDIO_PASS\n";
    long r; __asm__ volatile("syscall":"=a"(r):"0"(53L),"D"(1L),"S"(msg),"d"(11L):"rcx","r11","memory");
    return 0;
}
