/* selfhost_string.c -- string.h include test */
#include <string.h>
int main(void) {
    char buf[32];
    strcpy(buf, "hello");
    if (strlen(buf) != 5) return 1;
    const char msg[] = "STRING_PASS\n";
    long r; __asm__ volatile("syscall":"=a"(r):"0"(53L),"D"(1L),"S"(msg),"d"(12L):"rcx","r11","memory");
    return 0;
}
