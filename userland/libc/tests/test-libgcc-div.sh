#!/usr/bin/env bash
# Host check of the native libc's 64- and 128-bit division helpers
# (libgcc_rt.c: __udivdi3 .. __modti3) against the host's libgcc on random
# operands of every magnitude plus edge cases. Our symbols are renamed to
# v_* so the reference results really come from the host runtime.
# Usage: userland/libc/tests/test-libgcc-div.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cc -O2 -fno-builtin -Wall -Wextra -Werror -c -o "$work/rt.o" "$here/../src/libgcc_rt.c"
args=()
for f in udivdi3 umoddi3 divdi3 moddi3 udivti3 umodti3 divti3 modti3; do
    args+=(--redefine-sym "__$f=v_$f")
done
objcopy "${args[@]}" "$work/rt.o" "$work/rt2.o"
cat > "$work/t.c" <<'C'
#include <stdio.h>
#include <stdint.h>
typedef unsigned __int128 u128; typedef __int128 s128;
uint64_t v_udivdi3(uint64_t,uint64_t); uint64_t v_umoddi3(uint64_t,uint64_t);
int64_t v_divdi3(int64_t,int64_t); int64_t v_moddi3(int64_t,int64_t);
u128 v_udivti3(u128,u128); u128 v_umodti3(u128,u128); s128 v_divti3(s128,s128); s128 v_modti3(s128,s128);
static uint64_t x = 88172645463325252ULL;
static uint64_t rnd(void){ x^=x<<13; x^=x>>7; x^=x<<17; return x; }
static uint64_t shaped(void){ uint64_t v = rnd(); return v >> (rnd() % 64); }
int main(void){ long bad=0;
 for (int i=0;i<2000000;i++){
  uint64_t a=shaped(), b=shaped(); if(!b) b=1;
  if (v_udivdi3(a,b)!=a/b || v_umoddi3(a,b)!=a%b) bad++;
  int64_t sa=(int64_t)rnd()>>(rnd()%64), sb=(int64_t)rnd()>>(rnd()%64); if(!sb) sb=3;
  if (!(sa==INT64_MIN&&sb==-1) && (v_divdi3(sa,sb)!=sa/sb || v_moddi3(sa,sb)!=sa%sb)) bad++;
  u128 A=((u128)shaped()<<64|rnd())>>(rnd()%128), B=((u128)shaped()<<64|rnd())>>(rnd()%128); if(!B) B=7;
  if (v_udivti3(A,B)!=A/B || v_umodti3(A,B)!=A%B) bad++;
  s128 SA=(s128)A*((rnd()&1)?-1:1), SB=(s128)B*((rnd()&1)?-1:1);
  if (v_divti3(SA,SB)!=SA/SB || v_modti3(SA,SB)!=SA%SB) bad++;
 }
 uint64_t edge[][2]={{~0ULL,1},{1ULL<<63,1},{~0ULL,~0ULL},{~0ULL,3},{5,7}};
 for (unsigned i=0;i<5;i++) if(v_udivdi3(edge[i][0],edge[i][1])!=edge[i][0]/edge[i][1]) bad++;
 if (v_udivti3(~(u128)0,1)!=~(u128)0 || v_umodti3(~(u128)0,(u128)1<<127)!=~(u128)0>>1) bad++;
 printf("libgcc-div: %s (%ld mismatches)\n", bad?"FAIL":"all passed", bad); return bad!=0; }
C
cc -O1 -Wall -o "$work/t" "$work/t.c" "$work/rt2.o"
"$work/t"
