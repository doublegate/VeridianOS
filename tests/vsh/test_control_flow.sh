#!/bin/sh
# test_control_flow.sh -- Tests for control flow constructs in vsh
#
# Covers: if/then/else/elif/fi, while/do/done, until/do/done,
#         for/in/do/done, case/esac, nested loops, break, continue,
#         arithmetic for loops, select (basic).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Control Flow Tests ===\n\n"

# ---------------------------------------------------------------------------
# if/then/else/fi
# ---------------------------------------------------------------------------

_TEST_NAME="if: basic true branch"
_out=$($VSH -c 'if true; then echo yes; fi')
assert_equal "$_out" "yes"

_TEST_NAME="if: basic false branch"
_out=$($VSH -c 'if false; then echo yes; fi')
assert_equal "$_out" ""

_TEST_NAME="if/else: true takes then"
_out=$($VSH -c 'if true; then echo yes; else echo no; fi')
assert_equal "$_out" "yes"

_TEST_NAME="if/else: false takes else"
_out=$($VSH -c 'if false; then echo yes; else echo no; fi')
assert_equal "$_out" "no"

_TEST_NAME="if: test -z empty string"
_out=$($VSH -c 'if [ -z "" ]; then echo empty; fi')
assert_equal "$_out" "empty"

_TEST_NAME="if: test -n nonempty string"
_out=$($VSH -c 'if [ -n "hello" ]; then echo nonempty; fi')
assert_equal "$_out" "nonempty"

_TEST_NAME="if: test string equality"
_out=$($VSH -c 'if [ "abc" = "abc" ]; then echo match; fi')
assert_equal "$_out" "match"

_TEST_NAME="if: test string inequality"
_out=$($VSH -c 'if [ "abc" != "xyz" ]; then echo differ; fi')
assert_equal "$_out" "differ"

_TEST_NAME="if: test integer -eq"
_out=$($VSH -c 'if [ 42 -eq 42 ]; then echo equal; fi')
assert_equal "$_out" "equal"

_TEST_NAME="if: test integer -ne"
_out=$($VSH -c 'if [ 1 -ne 2 ]; then echo notequal; fi')
assert_equal "$_out" "notequal"

_TEST_NAME="if: test integer -lt"
_out=$($VSH -c 'if [ 1 -lt 2 ]; then echo less; fi')
assert_equal "$_out" "less"

_TEST_NAME="if: test integer -gt"
_out=$($VSH -c 'if [ 5 -gt 3 ]; then echo greater; fi')
assert_equal "$_out" "greater"

_TEST_NAME="if: test integer -le"
_out=$($VSH -c 'if [ 3 -le 3 ]; then echo ok; fi')
assert_equal "$_out" "ok"

_TEST_NAME="if: test integer -ge"
_out=$($VSH -c 'if [ 5 -ge 3 ]; then echo ok; fi')
assert_equal "$_out" "ok"

_TEST_NAME="if: command exit status"
_out=$($VSH -c 'if echo test > /dev/null; then echo ran; fi')
assert_equal "$_out" "ran"

# ---------------------------------------------------------------------------
# elif
# ---------------------------------------------------------------------------

_TEST_NAME="elif: first branch matches"
_out=$($VSH -c 'X=1; if [ $X -eq 1 ]; then echo one; elif [ $X -eq 2 ]; then echo two; fi')
assert_equal "$_out" "one"

_TEST_NAME="elif: second branch matches"
_out=$($VSH -c 'X=2; if [ $X -eq 1 ]; then echo one; elif [ $X -eq 2 ]; then echo two; fi')
assert_equal "$_out" "two"

_TEST_NAME="elif: else branch"
_out=$($VSH -c 'X=3; if [ $X -eq 1 ]; then echo one; elif [ $X -eq 2 ]; then echo two; else echo other; fi')
assert_equal "$_out" "other"

_TEST_NAME="elif: multiple elif"
_out=$($VSH -c 'X=3; if [ $X -eq 1 ]; then echo a; elif [ $X -eq 2 ]; then echo b; elif [ $X -eq 3 ]; then echo c; fi')
assert_equal "$_out" "c"

_TEST_NAME="elif: no match no else"
_out=$($VSH -c 'X=99; if [ $X -eq 1 ]; then echo a; elif [ $X -eq 2 ]; then echo b; fi')
assert_equal "$_out" ""

# ---------------------------------------------------------------------------
# Nested if
# ---------------------------------------------------------------------------

_TEST_NAME="if: nested if/then"
_out=$($VSH -c 'if true; then if true; then echo deep; fi; fi')
assert_equal "$_out" "deep"

_TEST_NAME="if: nested if/else"
_out=$($VSH -c 'if true; then if false; then echo no; else echo inner_else; fi; fi')
assert_equal "$_out" "inner_else"

_TEST_NAME="if: outer false skips inner"
_out=$($VSH -c 'if false; then if true; then echo inner; fi; fi')
assert_equal "$_out" ""

# ---------------------------------------------------------------------------
# while loop
# ---------------------------------------------------------------------------

_TEST_NAME="while: basic counting"
_out=$($VSH -c 'i=0; while [ $i -lt 3 ]; do echo $i; i=$((i+1)); done')
assert_contains "$_out" "0"

_TEST_NAME="while: counting to 3"
_out=$($VSH -c 'i=0; while [ $i -lt 3 ]; do echo $i; i=$((i+1)); done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="while: false condition never enters"
_out=$($VSH -c 'while false; do echo never; done')
assert_equal "$_out" ""

_TEST_NAME="while: accumulator"
_out=$($VSH -c 'sum=0; i=1; while [ $i -le 5 ]; do sum=$((sum+i)); i=$((i+1)); done; echo $sum')
assert_equal "$_out" "15"

_TEST_NAME="while: single iteration"
_out=$($VSH -c 'i=0; while [ $i -lt 1 ]; do echo once; i=$((i+1)); done')
assert_equal "$_out" "once"

# ---------------------------------------------------------------------------
# until loop
# ---------------------------------------------------------------------------

_TEST_NAME="until: basic counting"
_out=$($VSH -c 'i=0; until [ $i -ge 3 ]; do echo $i; i=$((i+1)); done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="until: true condition never enters"
_out=$($VSH -c 'until true; do echo never; done')
assert_equal "$_out" ""

_TEST_NAME="until: single iteration"
_out=$($VSH -c 'i=0; until [ $i -ge 1 ]; do echo once; i=$((i+1)); done')
assert_equal "$_out" "once"

# ---------------------------------------------------------------------------
# for/in loop
# ---------------------------------------------------------------------------

_TEST_NAME="for: iterate over words"
_out=$($VSH -c 'for x in a b c; do echo $x; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="for: first element"
_out=$($VSH -c 'for x in a b c; do echo $x; done')
_first=$(printf '%s\n' "$_out" | head -n 1)
assert_equal "$_first" "a"

_TEST_NAME="for: last element"
_out=$($VSH -c 'for x in a b c; do echo $x; done')
_last=$(printf '%s\n' "$_out" | tail -n 1)
assert_equal "$_last" "c"

_TEST_NAME="for: iterate over numbers"
_out=$($VSH -c 'for n in 1 2 3 4 5; do echo $n; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "5"

_TEST_NAME="for: empty word list"
_out=$($VSH -c 'for x in ; do echo $x; done')
assert_equal "$_out" ""

_TEST_NAME="for: single word"
_out=$($VSH -c 'for x in only; do echo $x; done')
assert_equal "$_out" "only"

_TEST_NAME="for: variable in word list"
_out=$($VSH -c 'LIST="x y z"; for item in $LIST; do echo $item; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="for: brace expansion in word list"
_out=$($VSH -c 'for i in {1..5}; do echo $i; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "5"

_TEST_NAME="for: nested for loops"
_out=$($VSH -c 'for a in 1 2; do for b in x y; do echo "$a$b"; done; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "4"

_TEST_NAME="for: nested with inner variable"
_out=$($VSH -c 'for a in 1 2; do for b in x y; do echo "$a$b"; done; done')
_first=$(printf '%s\n' "$_out" | head -n 1)
assert_equal "$_first" "1x"

# ---------------------------------------------------------------------------
# case/esac
# ---------------------------------------------------------------------------

_TEST_NAME="case: exact match"
_out=$($VSH -c 'X=hello; case $X in hello) echo matched;; esac')
assert_equal "$_out" "matched"

_TEST_NAME="case: no match"
_out=$($VSH -c 'X=foo; case $X in hello) echo matched;; esac')
assert_equal "$_out" ""

_TEST_NAME="case: wildcard pattern"
_out=$($VSH -c 'X=hello; case $X in h*) echo starts_h;; esac')
assert_equal "$_out" "starts_h"

_TEST_NAME="case: multiple patterns"
_out=$($VSH -c 'X=b; case $X in a) echo A;; b) echo B;; c) echo C;; esac')
assert_equal "$_out" "B"

_TEST_NAME="case: default with *"
_out=$($VSH -c 'X=unknown; case $X in a) echo A;; b) echo B;; *) echo DEFAULT;; esac')
assert_equal "$_out" "DEFAULT"

_TEST_NAME="case: or pattern with |"
_out=$($VSH -c 'X=yes; case $X in yes|y) echo affirm;; no|n) echo deny;; esac')
assert_equal "$_out" "affirm"

_TEST_NAME="case: empty value"
_out=$($VSH -c 'X=""; case $X in "") echo empty;; *) echo other;; esac')
assert_equal "$_out" "empty"

_TEST_NAME="case: number match"
_out=$($VSH -c 'X=42; case $X in 42) echo found;; esac')
assert_equal "$_out" "found"

_TEST_NAME="case: glob ? pattern"
_out=$($VSH -c 'X=ab; case $X in ?b) echo matched;; esac')
assert_equal "$_out" "matched"

_TEST_NAME="case: first match wins"
_out=$($VSH -c 'X=a; case $X in a) echo first;; a) echo second;; esac')
assert_equal "$_out" "first"

# ---------------------------------------------------------------------------
# break and continue
# ---------------------------------------------------------------------------

_TEST_NAME="break: exits while loop"
_out=$($VSH -c 'i=0; while true; do i=$((i+1)); if [ $i -ge 3 ]; then break; fi; echo $i; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "2"

_TEST_NAME="break: exits for loop"
_out=$($VSH -c 'for i in 1 2 3 4 5; do if [ $i -eq 3 ]; then break; fi; echo $i; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "2"

_TEST_NAME="continue: skips iteration in for"
_out=$($VSH -c 'for i in 1 2 3 4 5; do if [ $i -eq 3 ]; then continue; fi; echo $i; done')
assert_not_equal "$(printf '%s\n' "$_out" | grep 3)" "3" "3 should be skipped"

_TEST_NAME="continue: skips iteration in while"
_out=$($VSH -c 'i=0; while [ $i -lt 5 ]; do i=$((i+1)); if [ $i -eq 3 ]; then continue; fi; echo $i; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "4"

_TEST_NAME="break: after output"
_out=$($VSH -c 'for i in 1 2 3; do echo $i; if [ $i -eq 2 ]; then break; fi; done; echo done')
assert_contains "$_out" "done"

# ---------------------------------------------------------------------------
# Subshell
# ---------------------------------------------------------------------------

_TEST_NAME="subshell: basic execution"
_out=$($VSH -c '(echo hello)')
assert_equal "$_out" "hello"

_TEST_NAME="subshell: variable isolation"
_out=$($VSH -c 'X=outer; (X=inner; echo $X); echo $X')
_first=$(printf '%s\n' "$_out" | head -n 1)
_second=$(printf '%s\n' "$_out" | tail -n 1)
assert_equal "$_first" "inner"

_TEST_NAME="subshell: outer unchanged"
_out=$($VSH -c 'X=outer; (X=inner); echo $X')
assert_equal "$_out" "outer"

_TEST_NAME="subshell: exit status"
$VSH -c '(exit 42)'; _code=$?
assert_equal "$_code" "42"

_TEST_NAME="subshell: nested"
_out=$($VSH -c '((echo deep))')
assert_contains "$_out" "deep"

# ---------------------------------------------------------------------------
# Brace group
# ---------------------------------------------------------------------------

_TEST_NAME="brace: basic group"
_out=$($VSH -c '{ echo hello; echo world; }')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "2"

_TEST_NAME="brace: shares scope"
_out=$($VSH -c 'X=before; { X=after; }; echo $X')
assert_equal "$_out" "after"

_TEST_NAME="brace: with redirect"
_TMPF="${TMPDIR:-/tmp}/vsh_brace_$$.txt"
$VSH -c "{ echo a; echo b; } > $_TMPF"
_lines=$(wc -l < "$_TMPF")
assert_equal "$(echo $_lines)" "2"
rm -f "$_TMPF"

# ---------------------------------------------------------------------------
# Conditional expression: [[ ]]
# ---------------------------------------------------------------------------

_TEST_NAME="[[ ]]: string equality"
assert_exit_code 0 $VSH -c '[[ "hello" == "hello" ]]'

_TEST_NAME="[[ ]]: string inequality"
assert_exit_code 1 $VSH -c '[[ "hello" == "world" ]]'

_TEST_NAME="[[ ]]: not equal"
assert_exit_code 0 $VSH -c '[[ "a" != "b" ]]'

_TEST_NAME="[[ ]]: -z empty"
assert_exit_code 0 $VSH -c '[[ -z "" ]]'

_TEST_NAME="[[ ]]: -n nonempty"
assert_exit_code 0 $VSH -c '[[ -n "text" ]]'

_TEST_NAME="[[ ]]: and operator"
assert_exit_code 0 $VSH -c '[[ -n "a" && -n "b" ]]'

_TEST_NAME="[[ ]]: or operator"
assert_exit_code 0 $VSH -c '[[ -z "" || -n "b" ]]'

_TEST_NAME="[[ ]]: negation"
assert_exit_code 0 $VSH -c '[[ ! -z "text" ]]'

# ---------------------------------------------------------------------------
# Arithmetic evaluation: (( ))
# ---------------------------------------------------------------------------

_TEST_NAME="(( )): nonzero is true"
assert_exit_code 0 $VSH -c '(( 1 ))'

_TEST_NAME="(( )): zero is false"
assert_exit_code 1 $VSH -c '(( 0 ))'

_TEST_NAME="(( )): expression true"
assert_exit_code 0 $VSH -c '(( 2 + 2 == 4 ))'

_TEST_NAME="(( )): expression false"
assert_exit_code 1 $VSH -c '(( 2 + 2 == 5 ))'

_TEST_NAME="(( )): variable comparison"
assert_exit_code 0 $VSH -c 'x=10; (( x > 5 ))'

report
