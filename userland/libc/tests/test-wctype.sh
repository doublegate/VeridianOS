#!/usr/bin/env bash
# Host check of the native libc <wctype.h> (wctype.c): in the C locale the
# classification and case mapping of every ASCII character and of WEOF
# must agree with the host libc, nothing above U+007F belongs to a class
# or changes case, and wctype/iswctype and wctrans/towctrans must agree
# with the functions they name. Our symbols are renamed to v_* so the
# reference really is the host libc (PR #42 review).
# Usage: userland/libc/tests/test-wctype.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cc -O2 -Wall -Wextra -Werror -c -o "$work/wctype.o" "$here/../src/wctype.c"
fns="iswalnum iswalpha iswblank iswcntrl iswdigit iswgraph iswlower iswprint iswpunct iswspace
     iswupper iswxdigit towlower towupper wctype iswctype wctrans towctrans"
args=()
for f in $fns; do
    args+=(--redefine-sym "$f=v_$f")
done
objcopy "${args[@]}" "$work/wctype.o" "$work/wctype2.o"
cat > "$work/t.c" <<'C'
#include <locale.h>
#include <stdio.h>
#include <wctype.h>
#define V(f) int v_##f(wint_t);
V(iswalnum) V(iswalpha) V(iswblank) V(iswcntrl) V(iswdigit) V(iswgraph)
V(iswlower) V(iswprint) V(iswpunct) V(iswspace) V(iswupper) V(iswxdigit)
wint_t v_towlower(wint_t); wint_t v_towupper(wint_t);
wctype_t v_wctype(const char *); int v_iswctype(wint_t, wctype_t);
wctrans_t v_wctrans(const char *); wint_t v_towctrans(wint_t, wctrans_t);
typedef int (*cls)(wint_t);
int main(void) {
    setlocale(LC_ALL, "C");
    static const char *names[] = {"alnum", "alpha", "blank", "cntrl", "digit", "graph",
                                  "lower", "print", "punct", "space", "upper", "xdigit"};
    cls host[] = {iswalnum, iswalpha, iswblank, iswcntrl, iswdigit, iswgraph,
                  iswlower, iswprint, iswpunct, iswspace, iswupper, iswxdigit};
    cls ours[] = {v_iswalnum, v_iswalpha, v_iswblank, v_iswcntrl, v_iswdigit, v_iswgraph,
                  v_iswlower, v_iswprint, v_iswpunct, v_iswspace, v_iswupper, v_iswxdigit};
    int bad = 0;
    for (long c = -1; c < 0x80; c++) {
        wint_t w = c < 0 ? WEOF : (wint_t)c;
        for (int k = 0; k < 12; k++) {
            if (!host[k](w) != !ours[k](w)) { printf("FAIL isw%s(%#lx)\n", names[k], c); bad = 1; }
            if (!ours[k](w) != !v_iswctype(w, v_wctype(names[k]))) {
                printf("FAIL iswctype %s(%#lx)\n", names[k], c); bad = 1;
            }
        }
        if (towupper(w) != v_towupper(w) || towlower(w) != v_towlower(w) ||
            v_towctrans(w, v_wctrans("toupper")) != v_towupper(w) ||
            v_towctrans(w, v_wctrans("tolower")) != v_towlower(w)) {
            printf("FAIL case %#lx\n", c); bad = 1;
        }
    }
    static const wint_t wide[] = {0xe9, 0xc9, 0x3b1, 0x391, 0x2003, 0xff10, 0x10000, 0x10ffff};
    for (unsigned i = 0; i < sizeof(wide) / sizeof(wide[0]); i++) {
        for (int k = 0; k < 12; k++)
            if (ours[k](wide[i])) { printf("FAIL isw%s(%#x) in the C locale\n", names[k], wide[i]); bad = 1; }
        if (v_towupper(wide[i]) != wide[i] || v_towlower(wide[i]) != wide[i]) {
            printf("FAIL case %#x in the C locale\n", wide[i]); bad = 1;
        }
    }
    if (v_wctype("nonsense") != 0 || v_wctrans("nonsense") != 0) { printf("FAIL unknown names\n"); bad = 1; }
    if (!bad) printf("wctype: ok\n");
    return bad;
}
C
cc -O2 -Wall -Wextra -Werror -o "$work/t" "$work/t.c" "$work/wctype2.o"
"$work/t"
