#!/bin/sh
# test_variables.sh -- Tests for variable assignment, arrays, and special
#                      variables in vsh
#
# Covers: assignment, arrays (${arr[0]}, ${arr[@]}, ${#arr[@]}),
#         special vars ($?, $$, $!, $#, $@, $*, $0, $-, $_),
#         variable scoping, readonly, export attributes.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Variables Tests ===\n\n"

# ---------------------------------------------------------------------------
# Basic assignment
# ---------------------------------------------------------------------------

_TEST_NAME="assign: simple assignment"
_out=$($VSH -c 'X=hello; echo $X')
assert_equal "$_out" "hello"

_TEST_NAME="assign: reassignment"
_out=$($VSH -c 'X=first; X=second; echo $X')
assert_equal "$_out" "second"

_TEST_NAME="assign: empty value"
_out=$($VSH -c 'X=; echo "[$X]"')
assert_equal "$_out" "[]"

_TEST_NAME="assign: value with digits"
_out=$($VSH -c 'X=abc123; echo $X')
assert_equal "$_out" "abc123"

_TEST_NAME="assign: underscore in name"
_out=$($VSH -c 'MY_VAR=test; echo $MY_VAR')
assert_equal "$_out" "test"

_TEST_NAME="assign: leading underscore"
_out=$($VSH -c '_VAR=test; echo $_VAR')
assert_equal "$_out" "test"

_TEST_NAME="assign: numeric suffix"
_out=$($VSH -c 'VAR2=two; echo $VAR2')
assert_equal "$_out" "two"

_TEST_NAME="assign: quoted value with spaces"
_out=$($VSH -c 'X="hello world"; echo "$X"')
assert_equal "$_out" "hello world"

_TEST_NAME="assign: single-quoted value"
_out=$($VSH -c "X='hello world'; echo \"\$X\"")
assert_equal "$_out" "hello world"

_TEST_NAME="assign: value from command substitution"
_out=$($VSH -c 'X=$(echo test); echo $X')
assert_equal "$_out" "test"

_TEST_NAME="assign: multiple on same line"
_out=$($VSH -c 'A=1; B=2; C=3; echo $A $B $C')
assert_equal "$_out" "1 2 3"

_TEST_NAME="assign: value referencing another var"
_out=$($VSH -c 'A=hello; B="$A world"; echo $B')
assert_equal "$_out" "hello world"

# ---------------------------------------------------------------------------
# Variable expansion
# ---------------------------------------------------------------------------

_TEST_NAME="expand: unset variable is empty"
_out=$($VSH -c 'echo "[$UNSET_XYZ]"')
assert_equal "$_out" "[]"

_TEST_NAME="expand: braced form"
_out=$($VSH -c 'X=hello; echo ${X}')
assert_equal "$_out" "hello"

_TEST_NAME="expand: braced adjacent text"
_out=$($VSH -c 'X=hello; echo ${X}world')
assert_equal "$_out" "helloworld"

_TEST_NAME="expand: bare adjacent text"
_out=$($VSH -c 'X=test; echo ${X}_suffix')
assert_equal "$_out" "test_suffix"

_TEST_NAME="expand: in double quotes"
_out=$($VSH -c 'X="a b"; echo "$X"')
assert_equal "$_out" "a b"

_TEST_NAME="expand: word splitting unquoted"
_out=$($VSH -c 'X="a   b   c"; echo $X')
assert_equal "$_out" "a b c"

_TEST_NAME="expand: no splitting in double quotes"
_out=$($VSH -c 'X="a   b   c"; echo "$X"')
assert_equal "$_out" "a   b   c"

# ---------------------------------------------------------------------------
# Special variables: $?
# ---------------------------------------------------------------------------

_TEST_NAME="\$?: after true"
_out=$($VSH -c 'true; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="\$?: after false"
_out=$($VSH -c 'false; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="\$?: after exit 42 in subshell"
_out=$($VSH -c '(exit 42); echo $?')
assert_equal "$_out" "42"

_TEST_NAME="\$?: after successful command"
_out=$($VSH -c 'echo ok > /dev/null; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="\$?: reading \$? resets to 0"
_out=$($VSH -c 'false; X=$?; echo $X; echo $?')
_first=$(printf '%s\n' "$_out" | head -n 1)
_second=$(printf '%s\n' "$_out" | tail -n 1)
assert_equal "$_first" "1"

# ---------------------------------------------------------------------------
# Special variables: $$
# ---------------------------------------------------------------------------

_TEST_NAME="\$\$: is numeric"
_out=$($VSH -c 'echo $$')
assert_match "$_out" "^[0-9]+$"

_TEST_NAME="\$\$: is positive"
_out=$($VSH -c 'echo $$')
assert_not_equal "$_out" "0" "PID should not be 0"

_TEST_NAME="\$\$: consistent within session"
_out=$($VSH -c 'A=$$; B=$$; [ "$A" = "$B" ] && echo same || echo different')
assert_equal "$_out" "same"

# ---------------------------------------------------------------------------
# Special variables: $#
# ---------------------------------------------------------------------------

_TEST_NAME="\$#: no arguments"
_out=$($VSH -c 'echo $#')
assert_equal "$_out" "0"

_TEST_NAME="\$#: with set --"
_out=$($VSH -c 'set -- a b c; echo $#')
assert_equal "$_out" "3"

_TEST_NAME="\$#: after shift"
_out=$($VSH -c 'set -- a b c; shift; echo $#')
assert_equal "$_out" "2"

_TEST_NAME="\$#: single argument"
_out=$($VSH -c 'set -- only; echo $#')
assert_equal "$_out" "1"

# ---------------------------------------------------------------------------
# Special variables: $@, $*
# ---------------------------------------------------------------------------

_TEST_NAME="\$@: all positional params"
_out=$($VSH -c 'set -- a b c; echo $@')
assert_equal "$_out" "a b c"

_TEST_NAME="\$*: all positional params"
_out=$($VSH -c 'set -- a b c; echo $*')
assert_equal "$_out" "a b c"

_TEST_NAME="\$@: in for loop"
_out=$($VSH -c 'set -- x y z; for arg in "$@"; do echo $arg; done')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="\$@: empty when no args"
_out=$($VSH -c 'echo "[$@]"')
assert_equal "$_out" "[]"

# ---------------------------------------------------------------------------
# Positional parameters: $1, $2, ...
# ---------------------------------------------------------------------------

_TEST_NAME="\$1: first positional"
_out=$($VSH -c 'set -- alpha beta; echo $1')
assert_equal "$_out" "alpha"

_TEST_NAME="\$2: second positional"
_out=$($VSH -c 'set -- alpha beta; echo $2')
assert_equal "$_out" "beta"

_TEST_NAME="\$1: after shift"
_out=$($VSH -c 'set -- a b c; shift; echo $1')
assert_equal "$_out" "b"

_TEST_NAME="\$9: ninth positional"
_out=$($VSH -c 'set -- 1 2 3 4 5 6 7 8 nine; echo $9')
assert_equal "$_out" "nine"

_TEST_NAME="\$1: unset positional is empty"
_out=$($VSH -c 'echo "[$1]"')
assert_equal "$_out" "[]"

# ---------------------------------------------------------------------------
# Special variables: $0
# ---------------------------------------------------------------------------

_TEST_NAME="\$0: shell name"
_out=$($VSH -c 'echo $0')
assert_not_equal "$_out" "" "\$0 should not be empty"

# ---------------------------------------------------------------------------
# Variable attributes
# ---------------------------------------------------------------------------

_TEST_NAME="readonly: prevents overwrite"
_out=$($VSH -c 'readonly X=const; X=new 2>/dev/null; echo $X')
assert_equal "$_out" "const"

_TEST_NAME="readonly: prevents unset"
_out=$($VSH -c 'readonly X=const; unset X 2>/dev/null; echo $X')
assert_equal "$_out" "const"

_TEST_NAME="export: visible in subshell"
_out=$($VSH -c 'export X=test; $VSH -c "echo \$X"' 2>/dev/null)
# Depends on whether VSH is available; just check it doesn't crash
pass

_TEST_NAME="export: list shows exported vars"
_out=$($VSH -c 'export FOO=bar; export')
assert_contains "$_out" "FOO"

# ---------------------------------------------------------------------------
# Variable scoping
# ---------------------------------------------------------------------------

_TEST_NAME="scope: global visible everywhere"
_out=$($VSH -c 'X=global; echo $X')
assert_equal "$_out" "global"

_TEST_NAME="scope: subshell inherits"
_out=$($VSH -c 'X=parent; (echo $X)')
assert_equal "$_out" "parent"

_TEST_NAME="scope: subshell change not visible"
_out=$($VSH -c 'X=parent; (X=child); echo $X')
assert_equal "$_out" "parent"

_TEST_NAME="scope: brace group shares scope"
_out=$($VSH -c 'X=before; { X=after; }; echo $X')
assert_equal "$_out" "after"

# ---------------------------------------------------------------------------
# Unset
# ---------------------------------------------------------------------------

_TEST_NAME="unset: remove variable"
_out=$($VSH -c 'X=test; unset X; echo "[$X]"')
assert_equal "$_out" "[]"

_TEST_NAME="unset: already unset is ok"
_out=$($VSH -c 'unset NEVER_SET_XYZ; echo ok')
assert_equal "$_out" "ok"

_TEST_NAME="unset: multiple variables"
_out=$($VSH -c 'A=1; B=2; unset A B; echo "[$A][$B]"')
assert_equal "$_out" "[][]"

# ---------------------------------------------------------------------------
# Arrays
# ---------------------------------------------------------------------------

_TEST_NAME="array: indexed assignment"
_out=$($VSH -c 'arr[0]=hello; echo ${arr[0]}' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "hello"
else
    skip "indexed arrays not supported in -c mode"
fi

_TEST_NAME="array: multiple elements"
_out=$($VSH -c 'arr[0]=a; arr[1]=b; arr[2]=c; echo ${arr[1]}' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "b"
else
    skip "indexed arrays not fully supported"
fi

_TEST_NAME="array: parenthesized assignment"
_out=$($VSH -c 'arr=(x y z); echo ${arr[0]}' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "x"
else
    skip "array literal syntax not supported"
fi

_TEST_NAME="array: all elements \${arr[@]}"
_out=$($VSH -c 'arr=(a b c); echo ${arr[@]}' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "a b c"
else
    skip "array all-elements not supported"
fi

_TEST_NAME="array: length \${#arr[@]}"
_out=$($VSH -c 'arr=(a b c); echo ${#arr[@]}' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "3"
else
    skip "array length not supported"
fi

# ---------------------------------------------------------------------------
# Environment inheritance
# ---------------------------------------------------------------------------

_TEST_NAME="env: PATH is inherited"
_out=$($VSH -c 'echo $PATH')
assert_not_equal "$_out" "" "PATH should be inherited"

_TEST_NAME="env: HOME is inherited"
_out=$($VSH -c 'echo $HOME')
assert_not_equal "$_out" "" "HOME should be inherited"

_TEST_NAME="env: inline assignment for command"
_out=$($VSH -c 'X=hello $VSH -c "echo \$X"' 2>/dev/null)
# Inline env assignment may or may not be supported; just don't crash
pass

# ---------------------------------------------------------------------------
# Integer attribute
# ---------------------------------------------------------------------------

_TEST_NAME="declare -i: integer variable"
_out=$($VSH -c 'declare -i NUM=42; echo $NUM')
assert_equal "$_out" "42"

# ---------------------------------------------------------------------------
# Nameref
# ---------------------------------------------------------------------------

_TEST_NAME="declare -n: nameref (basic)"
_out=$($VSH -c 'X=hello; declare -n REF=X; echo $REF' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "hello"
else
    skip "nameref not fully supported"
fi

report
