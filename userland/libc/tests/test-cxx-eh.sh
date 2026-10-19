#!/usr/bin/env bash
# Host check of the native libc C++ exception runtime (cxa_exception.c +
# cxx_typeinfo.c): handlers are chosen by type (exact, public base through
# single, multiple and virtual inheritance, catch (...)), base-class handlers
# bind to the right subobject, and rethrow works. Links the runtime with the
# host unwinder and no libsupc++, so every catch goes through this code.
# Usage: userland/libc/tests/test-cxx-eh.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
src="$here/../src"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/t.cpp" <<'CPP'
#include <stdio.h>
struct Base { int b = 1; virtual ~Base() {} };
struct Other { int o = 2; virtual ~Other() {} };
struct Pad { long pad[3] = {7, 7, 7}; virtual ~Pad() {} };
struct Derived : Base { int d = 3; };
struct Multi : Pad, Base { int m = 4; };            // Base at a non-zero offset
struct VBase { int v = 5; virtual ~VBase() {} };
struct VMid : virtual VBase { int x = 6; };
struct VLeaf : Pad, VMid { int y = 8; };            // virtual base via the vtable
struct Priv : private Base { int p = 9; };
static int bad;
#define CHECK(c) do { if (!(c)) { printf("FAIL line %d: %s\n", __LINE__, #c); bad = 1; } } while (0)
template <class T> static void thrower(int k) { (void)k; throw T(); }
int main() {
    try { thrower<Derived>(0); } catch (Other &) { CHECK(0); } catch (Base &b) { CHECK(b.b == 1); }
    try { thrower<Multi>(0); } catch (Base &b) { CHECK(b.b == 1); }
    try { thrower<Multi>(0); } catch (Pad &p) { CHECK(p.pad[2] == 7); }
    try { thrower<VLeaf>(0); } catch (VBase &v) { CHECK(v.v == 5); }
    try { thrower<VLeaf>(0); } catch (VMid &m) { CHECK(m.x == 6); }
    try { thrower<Derived>(0); } catch (Derived &d) { CHECK(d.d == 3 && d.b == 1); }
    try { try { thrower<Priv>(0); } catch (Base &) { CHECK(0); } } catch (Priv &p) { CHECK(p.p == 9); }
    try { thrower<Other>(0); } catch (Base &) { CHECK(0); } catch (...) { CHECK(1); }
    try { try { thrower<Derived>(0); } catch (Derived &) { throw; } } catch (Base &b) { CHECK(b.b == 1); }
    puts(bad ? "cxx-eh: FAIL" : "cxx-eh: all passed");
    return bad;
}
#include <stdlib.h>
void operator delete(void *p, unsigned long) noexcept { free(p); }
void operator delete(void *p) noexcept { free(p); }
CPP
cc -c -O2 -Wall -Wextra -Werror -o "$work/eh.o" "$src/cxa_exception.c"
cc -c -O2 -Wall -Wextra -Werror -o "$work/ti.o" "$src/cxx_typeinfo.c"
c++ -O1 -Wall -c -o "$work/t.o" "$work/t.cpp"
c++ -o "$work/t" "$work/t.o" "$work/eh.o" "$work/ti.o" -nodefaultlibs -lgcc_eh -lgcc -lc
"$work/t"
