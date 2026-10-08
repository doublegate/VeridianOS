#!/usr/bin/env bash
# Host check of the glibc shim's __cxa_thread_atexit_impl: thread_local
# destructors run in reverse order at thread exit and, for the main
# thread, at exit(). The implementation is extracted from glibc_shim.c and
# linked into a C++ program, where it interposes on glibc's, so the host's
# libstdc++ calls it exactly as the cross-built KDE stack does.
# Usage: tools/cross/musl-compat/test-cxa-thread-atexit.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
{
    echo '#include <pthread.h>'
    echo '#include <stdlib.h>'
    awk '/^struct veridian_tls_dtor \{/{on=1} on{print} on&&/^int __cxa_thread_atexit_impl/{f=1} f&&/^}/{exit}' \
        "$here/glibc_shim.c"
} > "$work/impl.c"
grep -q '__cxa_thread_atexit_impl' "$work/impl.c" || { echo "FAIL: implementation not found" >&2; exit 1; }
cat > "$work/t.cpp" <<'CPP'
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static char order[64]; static int used;
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static void mark(char c) { pthread_mutex_lock(&m); order[used++] = c; pthread_mutex_unlock(&m); }
struct D { char c; D(char x) : c(x) {} ~D() { mark(c); } };
thread_local D a('a');
thread_local D b('b');
static void *worker(void *) { (void)a.c; (void)b.c; return nullptr; }  /* constructs a then b */
static void check_at_exit() {
    /* Worker thread: b then a (LIFO). Main thread: b then a too, already
     * run: the shim registers its atexit handler at the first thread_local
     * registration, after this one, and atexit handlers run in reverse. */
    int ok = strcmp(order, "baba") == 0;
    printf("cxa-thread-atexit: %s (order %s)\n", ok ? "all passed" : "FAIL", order);
    fflush(stdout);
    _exit(ok ? 0 : 1);
}
int main() {
    atexit(check_at_exit);
    pthread_t t; pthread_create(&t, nullptr, worker, nullptr); pthread_join(t, nullptr);
    (void)a.c; (void)b.c;
    return 0;
}
CPP
cc -O1 -Wall -Wextra -Werror -c -o "$work/impl.o" "$work/impl.c"
c++ -O1 -Wall -pthread -o "$work/t" "$work/t.cpp" "$work/impl.o"
"$work/t"
