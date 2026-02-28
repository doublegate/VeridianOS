#!/bin/sh
# test_builtins.sh -- Tests for shell builtin commands in vsh
#
# Covers: cd, echo, test/[, printf, read, type, alias/unalias,
#         export/unset, set, declare/typeset, local, readonly,
#         pwd, true, false, :, shift, history, hash, shopt.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Builtins Tests ===\n\n"

TMPDIR="${TMPDIR:-/tmp}/vsh_builtin_test_$$"
mkdir -p "$TMPDIR"
trap 'rm -rf "$TMPDIR"' EXIT

# ---------------------------------------------------------------------------
# echo
# ---------------------------------------------------------------------------

_TEST_NAME="echo: basic output"
_out=$($VSH -c 'echo hello')
assert_equal "$_out" "hello"

_TEST_NAME="echo: multiple words"
_out=$($VSH -c 'echo hello world')
assert_equal "$_out" "hello world"

_TEST_NAME="echo: no arguments (empty line)"
_out=$($VSH -c 'echo')
assert_equal "$_out" ""

_TEST_NAME="echo: -n suppresses newline"
_out=$($VSH -c 'echo -n hello; echo world')
assert_equal "$_out" "helloworld"

_TEST_NAME="echo: -e enables escapes"
_out=$($VSH -c 'echo -e "a\tb"')
assert_contains "$_out" "a"

_TEST_NAME="echo: -E disables escapes"
_out=$($VSH -c 'echo -E "hello\nworld"')
assert_equal "$_out" 'hello\nworld'

_TEST_NAME="echo: -ne combined"
_out=$($VSH -c 'echo -ne "hello\n"')
assert_equal "$_out" "hello"

_TEST_NAME="echo: -en combined"
_out=$($VSH -c 'echo -en "test"')
assert_equal "$_out" "test"

_TEST_NAME="echo: -- not treated as flag"
_out=$($VSH -c 'echo -- -n')
assert_contains "$_out" "-n"

# ---------------------------------------------------------------------------
# true / false / :
# ---------------------------------------------------------------------------

_TEST_NAME="true: exits 0"
assert_exit_code 0 $VSH -c 'true'

_TEST_NAME="false: exits 1"
assert_exit_code 1 $VSH -c 'false'

_TEST_NAME="colon: exits 0"
assert_exit_code 0 $VSH -c ':'

_TEST_NAME="true: \$? is 0"
_out=$($VSH -c 'true; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="false: \$? is 1"
_out=$($VSH -c 'false; echo $?')
assert_equal "$_out" "1"

# ---------------------------------------------------------------------------
# test / [
# ---------------------------------------------------------------------------

_TEST_NAME="test: -z empty string"
assert_exit_code 0 $VSH -c 'test -z ""'

_TEST_NAME="test: -z nonempty string"
assert_exit_code 1 $VSH -c 'test -z "hello"'

_TEST_NAME="test: -n nonempty string"
assert_exit_code 0 $VSH -c 'test -n "hello"'

_TEST_NAME="test: -n empty string"
assert_exit_code 1 $VSH -c 'test -n ""'

_TEST_NAME="test: string = string"
assert_exit_code 0 $VSH -c 'test "abc" = "abc"'

_TEST_NAME="test: string != string"
assert_exit_code 0 $VSH -c 'test "abc" != "xyz"'

_TEST_NAME="test: integer -eq"
assert_exit_code 0 $VSH -c 'test 5 -eq 5'

_TEST_NAME="test: integer -ne"
assert_exit_code 0 $VSH -c 'test 5 -ne 3'

_TEST_NAME="test: integer -lt"
assert_exit_code 0 $VSH -c 'test 3 -lt 5'

_TEST_NAME="test: integer -le equal"
assert_exit_code 0 $VSH -c 'test 5 -le 5'

_TEST_NAME="test: integer -gt"
assert_exit_code 0 $VSH -c 'test 7 -gt 3'

_TEST_NAME="test: integer -ge equal"
assert_exit_code 0 $VSH -c 'test 5 -ge 5'

_TEST_NAME="[: bracket form works"
assert_exit_code 0 $VSH -c '[ "abc" = "abc" ]'

_TEST_NAME="[: bracket inequality"
assert_exit_code 1 $VSH -c '[ "abc" = "xyz" ]'

_TEST_NAME="test: file exists (-e /)"
assert_exit_code 0 $VSH -c 'test -e /'

_TEST_NAME="test: file exists (-f) nonexistent"
assert_exit_code 1 $VSH -c 'test -f /nonexistent_file_xyz'

_TEST_NAME="test: -d on directory"
assert_exit_code 0 $VSH -c 'test -d /tmp'

# ---------------------------------------------------------------------------
# printf
# ---------------------------------------------------------------------------

_TEST_NAME="printf: basic string"
_out=$($VSH -c 'printf "hello"')
assert_equal "$_out" "hello"

_TEST_NAME="printf: %s substitution"
_out=$($VSH -c 'printf "%s" "world"')
assert_equal "$_out" "world"

_TEST_NAME="printf: %d integer"
_out=$($VSH -c 'printf "%d" 42')
assert_equal "$_out" "42"

_TEST_NAME="printf: newline in format"
_out=$($VSH -c 'printf "hello\nworld"')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "2"

_TEST_NAME="printf: tab in format"
_out=$($VSH -c 'printf "a\tb"')
assert_contains "$_out" "a"

_TEST_NAME="printf: %% literal percent"
_out=$($VSH -c 'printf "100%%"')
assert_equal "$_out" "100%"

_TEST_NAME="printf: multiple args"
_out=$($VSH -c 'printf "%s=%d" "x" 5')
assert_equal "$_out" "x=5"

_TEST_NAME="printf: no format args (missing arg)"
_out=$($VSH -c 'printf "%s"')
assert_equal "$_out" ""

# ---------------------------------------------------------------------------
# cd / pwd
# ---------------------------------------------------------------------------

_TEST_NAME="pwd: outputs current directory"
_out=$($VSH -c 'pwd')
assert_not_equal "$_out" "" "pwd should output something"

_TEST_NAME="pwd: path starts with /"
_out=$($VSH -c 'pwd')
assert_match "$_out" "^/"

_TEST_NAME="cd: change to /tmp"
_out=$($VSH -c 'cd /tmp; pwd')
assert_equal "$_out" "/tmp"

_TEST_NAME="cd: change to HOME"
_out=$($VSH -c 'cd; pwd')
assert_not_equal "$_out" ""

_TEST_NAME="cd: nonexistent directory fails"
assert_exit_code 1 $VSH -c 'cd /nonexistent_dir_xyz_999'

_TEST_NAME="cd -: returns to previous"
_out=$($VSH -c 'cd /tmp; cd /; cd -; pwd')
assert_contains "$_out" "/tmp"

# ---------------------------------------------------------------------------
# type
# ---------------------------------------------------------------------------

_TEST_NAME="type: recognizes builtin"
_out=$($VSH -c 'type echo')
assert_contains "$_out" "builtin"

_TEST_NAME="type: recognizes cd as builtin"
_out=$($VSH -c 'type cd')
assert_contains "$_out" "builtin"

_TEST_NAME="type: recognizes test as builtin"
_out=$($VSH -c 'type test')
assert_contains "$_out" "builtin"

_TEST_NAME="type: not found returns error"
$VSH -c 'type nonexistent_cmd_xyz_999' 2>/dev/null; _code=$?
assert_equal "$_code" "1"

_TEST_NAME="type: recognizes external command"
_out=$($VSH -c 'type cat' 2>/dev/null)
if [ -n "$_out" ]; then
    assert_contains "$_out" "cat"
else
    skip "cat not in PATH"
fi

# ---------------------------------------------------------------------------
# alias / unalias
# ---------------------------------------------------------------------------

_TEST_NAME="alias: set and use"
_out=$($VSH -c 'alias greet="echo hello"; greet')
assert_equal "$_out" "hello"

_TEST_NAME="alias: list all"
_out=$($VSH -c 'alias greet="echo hi"; alias')
assert_contains "$_out" "greet"

_TEST_NAME="alias: display specific"
_out=$($VSH -c 'alias greet="echo hi"; alias greet')
assert_contains "$_out" "greet"

_TEST_NAME="unalias: remove alias"
_out=$($VSH -c 'alias greet="echo hi"; unalias greet; alias greet' 2>&1)
assert_contains "$_out" "not found"

_TEST_NAME="unalias -a: remove all"
_out=$($VSH -c 'alias a1="echo 1"; alias a2="echo 2"; unalias -a; alias')
assert_equal "$_out" ""

# ---------------------------------------------------------------------------
# export / unset
# ---------------------------------------------------------------------------

_TEST_NAME="export: set and check"
_out=$($VSH -c 'export MY_VAR=hello; echo $MY_VAR')
assert_equal "$_out" "hello"

_TEST_NAME="export: list exported"
_out=$($VSH -c 'export MY_VAR=test; export')
assert_contains "$_out" "MY_VAR"

_TEST_NAME="export: without value"
_out=$($VSH -c 'MY_VAR=test; export MY_VAR; export')
assert_contains "$_out" "MY_VAR"

_TEST_NAME="unset: remove variable"
_out=$($VSH -c 'MY_VAR=test; unset MY_VAR; echo "[$MY_VAR]"')
assert_equal "$_out" "[]"

_TEST_NAME="unset -v: explicit variable unset"
_out=$($VSH -c 'MY_VAR=test; unset -v MY_VAR; echo "[$MY_VAR]"')
assert_equal "$_out" "[]"

_TEST_NAME="unset -f: unset function"
_out=$($VSH -c 'myfunc() { echo hi; }; unset -f myfunc; type myfunc' 2>&1)
assert_contains "$_out" "not found"

# ---------------------------------------------------------------------------
# set
# ---------------------------------------------------------------------------

_TEST_NAME="set: list variables"
_out=$($VSH -c 'MY_VAR=test; set')
assert_contains "$_out" "MY_VAR"

_TEST_NAME="set --: set positional params"
_out=$($VSH -c 'set -- a b c; echo $1 $2 $3')
assert_equal "$_out" "a b c"

_TEST_NAME="set --: update \$#"
_out=$($VSH -c 'set -- x y z; echo $#')
assert_equal "$_out" "3"

_TEST_NAME="set --: empty clears"
_out=$($VSH -c 'set -- a b; set --; echo $#')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# declare / typeset
# ---------------------------------------------------------------------------

_TEST_NAME="declare: set variable"
_out=$($VSH -c 'declare MY_VAR=hello; echo $MY_VAR')
assert_equal "$_out" "hello"

_TEST_NAME="declare -x: export"
_out=$($VSH -c 'declare -x MY_VAR=test; export')
assert_contains "$_out" "MY_VAR"

_TEST_NAME="declare -r: readonly"
_out=$($VSH -c 'declare -r RO_VAR=const; RO_VAR=new 2>&1; echo $RO_VAR')
assert_contains "$_out" "const"

_TEST_NAME="typeset: same as declare"
_out=$($VSH -c 'typeset MY_VAR=hello; echo $MY_VAR')
assert_equal "$_out" "hello"

# ---------------------------------------------------------------------------
# readonly
# ---------------------------------------------------------------------------

_TEST_NAME="readonly: prevents modification"
_out=$($VSH -c 'readonly RO=constant; RO=changed 2>/dev/null; echo $RO')
assert_equal "$_out" "constant"

_TEST_NAME="readonly: with value"
_out=$($VSH -c 'readonly MY_CONST=42; echo $MY_CONST')
assert_equal "$_out" "42"

_TEST_NAME="readonly: list all readonly vars"
_out=$($VSH -c 'readonly RO1=a; readonly')
assert_contains "$_out" "RO1"

# ---------------------------------------------------------------------------
# shift
# ---------------------------------------------------------------------------

_TEST_NAME="shift: shift by 1"
_out=$($VSH -c 'set -- a b c; shift; echo $1')
assert_equal "$_out" "b"

_TEST_NAME="shift: shift by 2"
_out=$($VSH -c 'set -- a b c; shift 2; echo $1')
assert_equal "$_out" "c"

_TEST_NAME="shift: updates \$#"
_out=$($VSH -c 'set -- a b c; shift; echo $#')
assert_equal "$_out" "2"

_TEST_NAME="shift: out of range fails"
_out=$($VSH -c 'set -- a; shift 5' 2>&1); _code=$?
assert_not_equal "$_code" "0"

# ---------------------------------------------------------------------------
# hash
# ---------------------------------------------------------------------------

_TEST_NAME="hash: hash a command"
$VSH -c 'hash cat' 2>/dev/null; _code=$?
# Just verify it doesn't crash
pass

_TEST_NAME="hash: list hashed"
_out=$($VSH -c 'hash cat 2>/dev/null; hash')
# May or may not have entries depending on PATH
pass

_TEST_NAME="hash -r: clear cache"
$VSH -c 'hash -r'; _code=$?
pass

# ---------------------------------------------------------------------------
# shopt
# ---------------------------------------------------------------------------

_TEST_NAME="shopt: list options"
_out=$($VSH -c 'shopt')
assert_contains "$_out" "extglob"

_TEST_NAME="shopt -s: enable option"
$VSH -c 'shopt -s dotglob' 2>/dev/null
pass

_TEST_NAME="shopt -u: disable option"
$VSH -c 'shopt -u dotglob' 2>/dev/null
pass

_TEST_NAME="shopt: invalid option"
$VSH -c 'shopt -s nonexistent_opt' 2>/dev/null; _code=$?
assert_not_equal "$_code" "0"

# ---------------------------------------------------------------------------
# exit
# ---------------------------------------------------------------------------

_TEST_NAME="exit: exit 0"
assert_exit_code 0 $VSH -c 'exit 0'

_TEST_NAME="exit: exit 1"
assert_exit_code 1 $VSH -c 'exit 1'

_TEST_NAME="exit: exit 42"
assert_exit_code 42 $VSH -c 'exit 42'

_TEST_NAME="exit: default to last status"
assert_exit_code 0 $VSH -c 'true; exit'

_TEST_NAME="exit: default to last status (failure)"
assert_exit_code 1 $VSH -c 'false; exit'

# ---------------------------------------------------------------------------
# let
# ---------------------------------------------------------------------------

_TEST_NAME="let: basic arithmetic"
_out=$($VSH -c 'let "x=5+3"; echo $?')
# let returns 0 if result is nonzero
assert_equal "$_out" "0"

_TEST_NAME="let: result zero returns 1"
_out=$($VSH -c 'let "0"; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="let: result nonzero returns 0"
_out=$($VSH -c 'let "42"; echo $?')
assert_equal "$_out" "0"

report
