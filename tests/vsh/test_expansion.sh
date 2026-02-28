#!/bin/sh
# test_expansion.sh -- Tests for brace, tilde, and parameter expansion in vsh
#
# Covers: {a,b,c}, {1..10}, {a..z}, {01..10..2}, tilde (~, ~+, ~-),
#         ${var:-default}, ${#var}, ${var%pat}, ${var/pat/rep},
#         ${var:offset:length}, ${var^}, ${var,,}, and more.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Expansion Tests ===\n\n"

# ---------------------------------------------------------------------------
# Brace expansion: comma-separated
# ---------------------------------------------------------------------------

_TEST_NAME="brace: simple comma list"
_out=$($VSH -c 'echo {a,b,c}')
assert_equal "$_out" "a b c"

_TEST_NAME="brace: prefix with brace"
_out=$($VSH -c 'echo file.{txt,sh,rs}')
assert_equal "$_out" "file.txt file.sh file.rs"

_TEST_NAME="brace: suffix with brace"
_out=$($VSH -c 'echo {pre,suf}fix')
assert_equal "$_out" "prefix suffix"

_TEST_NAME="brace: prefix and suffix"
_out=$($VSH -c 'echo /path/{a,b,c}/end')
assert_equal "$_out" "/path/a/end /path/b/end /path/c/end"

_TEST_NAME="brace: two elements"
_out=$($VSH -c 'echo {yes,no}')
assert_equal "$_out" "yes no"

_TEST_NAME="brace: empty alternative"
_out=$($VSH -c 'echo {,b}')
assert_equal "$_out" " b"

_TEST_NAME="brace: single element (no expansion)"
_out=$($VSH -c 'echo {single}')
assert_equal "$_out" "{single}"

_TEST_NAME="brace: nested braces"
_out=$($VSH -c 'echo {a,{b,c}}')
assert_equal "$_out" "a b c"

_TEST_NAME="brace: multiple expansions"
_out=$($VSH -c 'echo {a,b}{1,2}')
assert_equal "$_out" "a1 a2 b1 b2"

# ---------------------------------------------------------------------------
# Brace expansion: numeric sequences
# ---------------------------------------------------------------------------

_TEST_NAME="brace: numeric range ascending"
_out=$($VSH -c 'echo {1..5}')
assert_equal "$_out" "1 2 3 4 5"

_TEST_NAME="brace: numeric range descending"
_out=$($VSH -c 'echo {5..1}')
assert_equal "$_out" "5 4 3 2 1"

_TEST_NAME="brace: numeric range with step"
_out=$($VSH -c 'echo {0..10..2}')
assert_equal "$_out" "0 2 4 6 8 10"

_TEST_NAME="brace: numeric range with step 3"
_out=$($VSH -c 'echo {1..10..3}')
assert_equal "$_out" "1 4 7 10"

_TEST_NAME="brace: zero-padded sequence"
_out=$($VSH -c 'echo {01..05}')
assert_equal "$_out" "01 02 03 04 05"

_TEST_NAME="brace: negative range"
_out=$($VSH -c 'echo {-3..3}')
assert_equal "$_out" "-3 -2 -1 0 1 2 3"

_TEST_NAME="brace: single number (no expansion)"
_out=$($VSH -c 'echo {5..5}')
assert_equal "$_out" "5"

_TEST_NAME="brace: prefix with numeric sequence"
_out=$($VSH -c 'echo file{1..3}.txt')
assert_equal "$_out" "file1.txt file2.txt file3.txt"

# ---------------------------------------------------------------------------
# Brace expansion: character sequences
# ---------------------------------------------------------------------------

_TEST_NAME="brace: char range a-e"
_out=$($VSH -c 'echo {a..e}')
assert_equal "$_out" "a b c d e"

_TEST_NAME="brace: char range A-E"
_out=$($VSH -c 'echo {A..E}')
assert_equal "$_out" "A B C D E"

_TEST_NAME="brace: char range descending z-v"
_out=$($VSH -c 'echo {z..v}')
assert_equal "$_out" "z y x w v"

_TEST_NAME="brace: char range with step"
_out=$($VSH -c 'echo {a..z..5}')
assert_equal "$_out" "a f k p u z"

# ---------------------------------------------------------------------------
# Tilde expansion
# ---------------------------------------------------------------------------

_TEST_NAME="tilde: bare ~ expands to HOME"
_out=$($VSH -c 'echo ~')
assert_not_equal "$_out" "~" "tilde should expand"

_TEST_NAME="tilde: ~/path expands"
_out=$($VSH -c 'echo ~/Documents')
assert_match "$_out" ".*/Documents$"

_TEST_NAME="tilde: ~+ expands to PWD"
_out=$($VSH -c 'echo ~+')
assert_not_equal "$_out" "~+"

_TEST_NAME="tilde: ~- expands to OLDPWD"
_out=$($VSH -c 'export OLDPWD=/tmp; echo ~-')
assert_equal "$_out" "/tmp"

_TEST_NAME="tilde: quoted tilde is literal"
_out=$($VSH -c "echo '~'")
assert_equal "$_out" "~"

_TEST_NAME="tilde: tilde in middle of word is literal"
_out=$($VSH -c 'echo foo~bar')
assert_equal "$_out" "foo~bar"

# ---------------------------------------------------------------------------
# Parameter expansion: basic
# ---------------------------------------------------------------------------

_TEST_NAME="param: simple \$VAR"
_out=$($VSH -c 'X=hello; echo $X')
assert_equal "$_out" "hello"

_TEST_NAME="param: braced \${VAR}"
_out=$($VSH -c 'X=hello; echo ${X}')
assert_equal "$_out" "hello"

_TEST_NAME="param: braced with suffix text"
_out=$($VSH -c 'X=hello; echo ${X}_world')
assert_equal "$_out" "hello_world"

_TEST_NAME="param: unset variable is empty"
_out=$($VSH -c 'echo "[$UNSET_VAR]"')
assert_equal "$_out" "[]"

_TEST_NAME="param: variable with digits in name"
_out=$($VSH -c 'VAR123=abc; echo $VAR123')
assert_equal "$_out" "abc"

_TEST_NAME="param: variable with underscore"
_out=$($VSH -c 'MY_VAR=test; echo $MY_VAR')
assert_equal "$_out" "test"

# ---------------------------------------------------------------------------
# Parameter expansion: default values
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var:-default} when unset"
_out=$($VSH -c 'echo ${UNSET:-fallback}')
assert_equal "$_out" "fallback"

_TEST_NAME="param: \${var:-default} when empty"
_out=$($VSH -c 'X=""; echo ${X:-fallback}')
assert_equal "$_out" "fallback"

_TEST_NAME="param: \${var:-default} when set"
_out=$($VSH -c 'X=value; echo ${X:-fallback}')
assert_equal "$_out" "value"

_TEST_NAME="param: \${var:+alternate} when set"
_out=$($VSH -c 'X=value; echo ${X:+alternate}')
assert_equal "$_out" "alternate"

_TEST_NAME="param: \${var:+alternate} when unset"
_out=$($VSH -c 'echo ${UNSET:+alternate}')
assert_equal "$_out" ""

_TEST_NAME="param: \${var:=assign} when unset"
_out=$($VSH -c 'echo ${UNSET:=assigned}')
assert_equal "$_out" "assigned"

# ---------------------------------------------------------------------------
# Parameter expansion: string length
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${#var} length"
_out=$($VSH -c 'X=hello; echo ${#X}')
assert_equal "$_out" "5"

_TEST_NAME="param: \${#var} empty string"
_out=$($VSH -c 'X=""; echo ${#X}')
assert_equal "$_out" "0"

_TEST_NAME="param: \${#var} unset variable"
_out=$($VSH -c 'echo ${#UNSET}')
assert_equal "$_out" "0"

_TEST_NAME="param: \${#var} longer string"
_out=$($VSH -c 'X="hello world"; echo ${#X}')
assert_equal "$_out" "11"

# ---------------------------------------------------------------------------
# Parameter expansion: suffix removal
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var%pat} shortest suffix"
_out=$($VSH -c 'X=file.tar.gz; echo ${X%.gz}')
assert_equal "$_out" "file.tar"

_TEST_NAME="param: \${var%%pat} longest suffix"
_out=$($VSH -c 'X=file.tar.gz; echo ${X%%.*}')
assert_equal "$_out" "file"

_TEST_NAME="param: \${var%pat} with glob"
_out=$($VSH -c 'X=/home/user/file.txt; echo ${X%/*}')
assert_equal "$_out" "/home/user"

_TEST_NAME="param: \${var%%pat} with glob"
_out=$($VSH -c 'X=/home/user/file.txt; echo ${X%%/*}')
assert_equal "$_out" ""

_TEST_NAME="param: \${var%pat} no match"
_out=$($VSH -c 'X=hello; echo ${X%.xyz}')
assert_equal "$_out" "hello"

# ---------------------------------------------------------------------------
# Parameter expansion: prefix removal
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var#pat} shortest prefix"
_out=$($VSH -c 'X=/home/user/file.txt; echo ${X#*/}')
assert_equal "$_out" "home/user/file.txt"

_TEST_NAME="param: \${var##pat} longest prefix"
_out=$($VSH -c 'X=/home/user/file.txt; echo ${X##*/}')
assert_equal "$_out" "file.txt"

_TEST_NAME="param: \${var#pat} no match"
_out=$($VSH -c 'X=hello; echo ${X#xyz}')
assert_equal "$_out" "hello"

_TEST_NAME="param: \${var##pat} basename extraction"
_out=$($VSH -c 'X=/usr/local/bin/vsh; echo ${X##*/}')
assert_equal "$_out" "vsh"

# ---------------------------------------------------------------------------
# Parameter expansion: substitution
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var/pat/rep} single replacement"
_out=$($VSH -c 'X=hello; echo ${X/l/L}')
assert_equal "$_out" "heLlo"

_TEST_NAME="param: \${var//pat/rep} global replacement"
_out=$($VSH -c 'X=hello; echo ${X//l/L}')
assert_equal "$_out" "heLLo"

_TEST_NAME="param: \${var/pat/rep} no match"
_out=$($VSH -c 'X=hello; echo ${X/z/Z}')
assert_equal "$_out" "hello"

_TEST_NAME="param: \${var/pat/} deletion"
_out=$($VSH -c 'X=hello; echo ${X/l/}')
assert_equal "$_out" "helo"

_TEST_NAME="param: \${var//pat/} global deletion"
_out=$($VSH -c 'X=hello; echo ${X//l/}')
assert_equal "$_out" "heo"

# ---------------------------------------------------------------------------
# Parameter expansion: case modification
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var^} uppercase first"
_out=$($VSH -c 'X=hello; echo ${X^}')
assert_equal "$_out" "Hello"

_TEST_NAME="param: \${var^^} uppercase all"
_out=$($VSH -c 'X=hello; echo ${X^^}')
assert_equal "$_out" "HELLO"

_TEST_NAME="param: \${var,} lowercase first"
_out=$($VSH -c 'X=HELLO; echo ${X,}')
assert_equal "$_out" "hELLO"

_TEST_NAME="param: \${var,,} lowercase all"
_out=$($VSH -c 'X=HELLO; echo ${X,,}')
assert_equal "$_out" "hello"

_TEST_NAME="param: case mod on empty string"
_out=$($VSH -c 'X=""; echo ${X^^}')
assert_equal "$_out" ""

# ---------------------------------------------------------------------------
# Parameter expansion: substring
# ---------------------------------------------------------------------------

_TEST_NAME="param: \${var:offset} from position"
_out=$($VSH -c 'X=hello; echo ${X:2}')
assert_equal "$_out" "llo"

_TEST_NAME="param: \${var:offset:length} substring"
_out=$($VSH -c 'X=hello; echo ${X:1:3}')
assert_equal "$_out" "ell"

_TEST_NAME="param: \${var:negative} from end"
_out=$($VSH -c 'X=hello; echo ${X: -2}')
assert_equal "$_out" "lo"

_TEST_NAME="param: \${var:0:0} empty substring"
_out=$($VSH -c 'X=hello; echo ${X:0:0}')
assert_equal "$_out" ""

_TEST_NAME="param: \${var:0} full string"
_out=$($VSH -c 'X=hello; echo ${X:0}')
assert_equal "$_out" "hello"

# ---------------------------------------------------------------------------
# Arithmetic expansion
# ---------------------------------------------------------------------------

_TEST_NAME="arith: basic addition in \$(())"
_out=$($VSH -c 'echo $((1+2))')
assert_equal "$_out" "3"

_TEST_NAME="arith: inside double quotes"
_out=$($VSH -c 'echo "total: $((10*5))"')
assert_equal "$_out" "total: 50"

_TEST_NAME="arith: variable reference"
_out=$($VSH -c 'X=10; echo $((X+5))')
assert_equal "$_out" "15"

_TEST_NAME="arith: nested in parameter expansion"
_out=$($VSH -c 'X=hello; echo ${X:$((1+1)):2}')
assert_equal "$_out" "ll"

# ---------------------------------------------------------------------------
# Special variables
# ---------------------------------------------------------------------------

_TEST_NAME="special: \$? exit status"
_out=$($VSH -c 'true; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="special: \$? after false"
_out=$($VSH -c 'false; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="special: \$\$ is numeric"
_out=$($VSH -c 'echo $$')
assert_match "$_out" "^[0-9]+$"

_TEST_NAME="special: \$# with no args"
_out=$($VSH -c 'echo $#')
assert_equal "$_out" "0"

report
