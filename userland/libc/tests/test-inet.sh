#!/usr/bin/env bash
# Host check of the native libc address conversions (inet.c): inet_aton,
# inet_addr, inet_pton and inet_ntop for AF_INET and AF_INET6 must agree
# with the host libc on edge-case inputs and on random addresses, and
# inet_ntop output must parse back to the same address. Our symbols are
# renamed to v_* so the reference really is the host libc.
# Usage: userland/libc/tests/test-inet.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cc -O2 -Wall -Wextra -Werror -c -o "$work/inet.o" "$here/../src/inet.c"
args=()
for f in inet_aton inet_addr inet_ntoa inet_pton inet_ntop; do
    args+=(--redefine-sym "$f=v_$f")
done
objcopy "${args[@]}" "$work/inet.o" "$work/inet2.o"
cat > "$work/t.c" <<'C'
#include <arpa/inet.h>
#include <stdio.h>
#include <string.h>
int v_inet_aton(const char *, struct in_addr *); in_addr_t v_inet_addr(const char *);
int v_inet_pton(int, const char *, void *); const char *v_inet_ntop(int, const void *, char *, socklen_t);
static unsigned x = 12345; static unsigned rnd(void) { x ^= x << 13; x ^= x >> 17; x ^= x << 5; return x; }
int main(void) {
    static const char *in[] = { "1.2.3.4", "0x7f.1", "127.1", "1.2.65535", "010.0.0.1", "0", "0x", "1..2", "1.2.3.4.5",
        "256.1.1.1", "1.2.3.4 junk", "1.2.3.4x", "4294967295", "4294967296", "0x100.0.0.1", "", ".", "1.2.3.", " 1.2.3.4",
        "08.1.1.1", "0777.0.0.1", "1.2.3.04", "01.2.3.4", "::", "::1", "1::", "1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7::",
        "::1:2:3:4:5:6:7", "1:2:3:4:5:6:7:8:9", ":::", "1:::2", "::ffff:1.2.3.4", "::1.2.3.4", "1:2:3:4:5:6:1.2.3.4",
        "1:2:3:4:5:6:7:1.2.3.4", "fe80::1%eth0", "12345::", "1:2", ":1:2:3:4:5:6:7", "1:2:3:4:5:6:7:", "abcd:EF01::",
        "::1.2.3", "::256.1.1.1", "1::2::3", "0:0:0:0:0:0:0:0", "2001:db8::1:0:0:1", "a:b:c:d:e:f:0:0", 0 };
    int bad = 0;
    for (int i = 0; in[i]; i++) {
        struct in_addr a1, a2; unsigned char b1[16], b2[16]; memset(&a1, 0, 4); memset(&a2, 0, 4);
        int r1 = inet_aton(in[i], &a1), r2 = v_inet_aton(in[i], &a2);
        if (r1 != r2 || (r1 && a1.s_addr != a2.s_addr)) { printf("FAIL aton %s: %d/%d\n", in[i], r1, r2); bad = 1; }
        if (inet_addr(in[i]) != v_inet_addr(in[i])) { printf("FAIL addr %s\n", in[i]); bad = 1; }
        for (int af = AF_INET; af <= AF_INET6; af += AF_INET6 - AF_INET) {
            memset(b1, 0, 16); memset(b2, 0, 16);
            r1 = inet_pton(af, in[i], b1); r2 = v_inet_pton(af, in[i], b2);
            if (r1 != r2 || (r1 == 1 && memcmp(b1, b2, 16))) { printf("FAIL pton af=%d %s: %d/%d\n", af, in[i], r1, r2); bad = 1; }
        }
    }
    for (int i = 0; i < 200000; i++) {
        unsigned char a[16]; char s1[64], s2[64];
        for (int k = 0; k < 16; k++) { unsigned r = rnd(); a[k] = (r & 3) ? 0 : (unsigned char)(r >> 8); }
        if (i % 7 == 0) { memset(a, 0, 10); a[10] = a[11] = 0xff; }
        if (i % 11 == 0) memset(a, 0, 12);
        int af = (i & 1) ? AF_INET6 : AF_INET;
        inet_ntop(af, a, s1, sizeof s1); v_inet_ntop(af, a, s2, sizeof s2);
        if (strcmp(s1, s2)) { printf("FAIL ntop %s vs %s\n", s1, s2); bad = 1; break; }
        unsigned char back[16]; memset(back, 0, 16);
        if (v_inet_pton(af, s2, back) != 1 || memcmp(back, a, af == AF_INET ? 4 : 16)) { printf("FAIL roundtrip %s\n", s2); bad = 1; break; }
    }
    char small[8]; unsigned char lo[16] = {0}; lo[15] = 1;
    if (v_inet_ntop(AF_INET6, lo, small, 3) != NULL || v_inet_ntop(AF_INET6, lo, small, 4) == NULL) { puts("FAIL size"); bad = 1; }
    puts(bad ? "inet: FAIL" : "inet: all passed");
    return bad;
}
C
cc -O1 -Wall -o "$work/t" "$work/t.c" "$work/inet2.o"
"$work/t"
