#!/bin/sh
# test_quoting.sh -- Tests for quoting and escaping in vsh
#
# Covers: single quotes, double quotes, ANSI-C quoting ($''),
#         backslash escaping, nested quotes, quote removal.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Quoting Tests ===\n\n"

# ---------------------------------------------------------------------------
# Single quotes
# ---------------------------------------------------------------------------

_TEST_NAME="single quote: literal string"
_out=$($VSH -c "echo 'hello world'")
assert_equal "$_out" "hello world"

_TEST_NAME="single quote: preserves special chars"
_out=$($VSH -c "echo 'hello \$HOME'")
assert_equal "$_out" 'hello $HOME'

_TEST_NAME="single quote: preserves backslash"
_out=$($VSH -c "echo 'hello\\nworld'")
assert_equal "$_out" 'hello\nworld'

_TEST_NAME="single quote: preserves double quotes inside"
_out=$($VSH -c "echo 'he said \"hi\"'")
assert_equal "$_out" 'he said "hi"'

_TEST_NAME="single quote: empty string"
_out=$($VSH -c "echo ''")
assert_equal "$_out" ""

_TEST_NAME="single quote: adjacent single-quoted strings"
_out=$($VSH -c "echo 'hello'' world'")
assert_equal "$_out" "hello world"

_TEST_NAME="single quote: preserves glob characters"
_out=$($VSH -c "echo 'file*.txt'")
assert_equal "$_out" "file*.txt"

_TEST_NAME="single quote: preserves pipe character"
_out=$($VSH -c "echo 'a | b'")
assert_equal "$_out" "a | b"

_TEST_NAME="single quote: preserves semicolon"
_out=$($VSH -c "echo 'a; b'")
assert_equal "$_out" "a; b"

_TEST_NAME="single quote: preserves backtick"
_out=$($VSH -c "echo 'hello \`world\`'")
assert_equal "$_out" 'hello `world`'

# ---------------------------------------------------------------------------
# Double quotes
# ---------------------------------------------------------------------------

_TEST_NAME="double quote: literal string"
_out=$($VSH -c 'echo "hello world"')
assert_equal "$_out" "hello world"

_TEST_NAME="double quote: variable expansion"
_out=$($VSH -c 'FOO=bar; echo "value: $FOO"')
assert_equal "$_out" "value: bar"

_TEST_NAME="double quote: preserves spaces in variable"
_out=$($VSH -c 'FOO="hello   world"; echo "$FOO"')
assert_equal "$_out" "hello   world"

_TEST_NAME="double quote: escaped dollar sign"
_out=$($VSH -c 'echo "price: \$5"')
assert_equal "$_out" 'price: $5'

_TEST_NAME="double quote: escaped double quote"
_out=$($VSH -c 'echo "he said \"hi\""')
assert_equal "$_out" 'he said "hi"'

_TEST_NAME="double quote: escaped backslash"
_out=$($VSH -c 'echo "path\\dir"')
assert_equal "$_out" 'path\dir'

_TEST_NAME="double quote: preserves single quote inside"
_out=$($VSH -c "echo \"it's fine\"")
assert_equal "$_out" "it's fine"

_TEST_NAME="double quote: empty string"
_out=$($VSH -c 'echo ""')
assert_equal "$_out" ""

_TEST_NAME="double quote: preserves leading/trailing whitespace"
_out=$($VSH -c 'echo "  spaced  "')
assert_equal "$_out" "  spaced  "

_TEST_NAME="double quote: multiple words without splitting"
_out=$($VSH -c 'X="a b c"; echo "$X"')
assert_equal "$_out" "a b c"

_TEST_NAME="double quote: arithmetic expansion inside"
_out=$($VSH -c 'echo "result: $((2+3))"')
assert_equal "$_out" "result: 5"

_TEST_NAME="double quote: nested parameter expansion"
_out=$($VSH -c 'X=hello; echo "len: ${#X}"')
assert_equal "$_out" "len: 5"

# ---------------------------------------------------------------------------
# Backslash escaping (unquoted)
# ---------------------------------------------------------------------------

_TEST_NAME="backslash: escape space"
_out=$($VSH -c 'echo hello\ world')
assert_equal "$_out" "hello world"

_TEST_NAME="backslash: escape dollar sign"
_out=$($VSH -c 'echo \$HOME')
assert_equal "$_out" '$HOME'

_TEST_NAME="backslash: escape backslash"
_out=$($VSH -c 'echo \\\\')
assert_equal "$_out" '\\'

_TEST_NAME="backslash: escape single quote"
_out=$($VSH -c "echo it\\'s")
assert_equal "$_out" "it's"

_TEST_NAME="backslash: escape hash"
_out=$($VSH -c 'echo \#comment')
assert_equal "$_out" "#comment"

_TEST_NAME="backslash: escape glob star"
_out=$($VSH -c 'echo \*.txt')
assert_equal "$_out" "*.txt"

_TEST_NAME="backslash: escape semicolon"
_out=$($VSH -c 'echo hello\; echo world')
assert_contains "$_out" "hello;"

_TEST_NAME="backslash: escape pipe"
_out=$($VSH -c 'echo hello\|world')
assert_equal "$_out" "hello|world"

# ---------------------------------------------------------------------------
# Echo -e escape sequences
# ---------------------------------------------------------------------------

_TEST_NAME="echo -e: newline escape"
_out=$($VSH -c 'echo -e "hello\nworld"')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$_lines" "2"

_TEST_NAME="echo -e: tab escape"
_out=$($VSH -c 'echo -e "col1\tcol2"')
assert_contains "$_out" "col1"

_TEST_NAME="echo -e: backslash escape"
_out=$($VSH -c 'echo -e "a\\\\b"')
assert_equal "$_out" 'a\b'

_TEST_NAME="echo -e: carriage return"
_out=$($VSH -c 'echo -e "hello\rworld"')
assert_contains "$_out" "world"

_TEST_NAME="echo -e: alert (bell)"
_out=$($VSH -c 'echo -e "x\ay"')
assert_contains "$_out" "x"

_TEST_NAME="echo -e: octal escape"
_out=$($VSH -c 'echo -e "\0101"')
assert_equal "$_out" "A"

_TEST_NAME="echo -e: hex escape"
_out=$($VSH -c 'echo -e "\x41"')
assert_equal "$_out" "A"

# ---------------------------------------------------------------------------
# Echo -n flag
# ---------------------------------------------------------------------------

_TEST_NAME="echo -n: no trailing newline"
_out=$($VSH -c 'echo -n hello; echo " world"')
assert_equal "$_out" "hello world"

_TEST_NAME="echo -n -e: combined flags"
_out=$($VSH -c 'echo -ne "hello\t"')
assert_contains "$_out" "hello"

_TEST_NAME="echo -en: combined short flags"
_out=$($VSH -c 'echo -en "test"')
assert_equal "$_out" "test"

_TEST_NAME="echo -E: disable escape interpretation"
_out=$($VSH -c 'echo -E "hello\nworld"')
assert_equal "$_out" 'hello\nworld'

# ---------------------------------------------------------------------------
# Mixed quoting
# ---------------------------------------------------------------------------

_TEST_NAME="mixed: single then double"
_out=$($VSH -c "echo 'hello '\"world\"")
assert_equal "$_out" "hello world"

_TEST_NAME="mixed: double then single"
_out=$($VSH -c "echo \"hello \"'world'")
assert_equal "$_out" "hello world"

_TEST_NAME="mixed: variable in unquoted part"
_out=$($VSH -c "X=test; echo 'prefix_'\$X")
assert_equal "$_out" "prefix_test"

_TEST_NAME="mixed: variable in double quoted part"
_out=$($VSH -c "X=test; echo 'prefix_'\"_\$X\"")
assert_equal "$_out" "prefix___test"

_TEST_NAME="mixed: adjacent quoted strings concatenate"
_out=$($VSH -c "echo 'abc'\"def\"'ghi'")
assert_equal "$_out" "abcdefghi"

# ---------------------------------------------------------------------------
# Quote edge cases
# ---------------------------------------------------------------------------

_TEST_NAME="edge: empty single quotes produce empty word"
_out=$($VSH -c "echo a '' b")
assert_equal "$_out" "a  b"

_TEST_NAME="edge: empty double quotes produce empty word"
_out=$($VSH -c 'echo a "" b')
assert_equal "$_out" "a  b"

_TEST_NAME="edge: dollar sign at end of double quote"
_out=$($VSH -c 'echo "cost$"')
assert_equal "$_out" 'cost$'

_TEST_NAME="edge: lone backslash at end (unquoted)"
_out=$($VSH -c 'echo test\\')
assert_contains "$_out" "test"

_TEST_NAME="edge: multiple spaces in double quotes preserved"
_out=$($VSH -c 'echo "a    b    c"')
assert_equal "$_out" "a    b    c"

_TEST_NAME="edge: newline in double-quoted string"
_out=$($VSH -c 'echo "line1
line2"')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$_lines" "2"

# ---------------------------------------------------------------------------
# Quoting in assignments
# ---------------------------------------------------------------------------

_TEST_NAME="assign: single-quoted value"
_out=$($VSH -c "X='hello world'; echo \$X")
assert_equal "$_out" "hello world"

_TEST_NAME="assign: double-quoted value with expansion"
_out=$($VSH -c 'A=hello; B="value: $A"; echo $B')
assert_equal "$_out" "value: hello"

_TEST_NAME="assign: empty string assignment"
_out=$($VSH -c 'X=""; echo "[$X]"')
assert_equal "$_out" "[]"

_TEST_NAME="assign: space in value needs quotes"
_out=$($VSH -c 'X="a b"; echo "$X"')
assert_equal "$_out" "a b"

_TEST_NAME="assign: special chars in value"
_out=$($VSH -c "X='!@#\$%^&*()'; echo \"\$X\"")
assert_contains "$_out" "!@#"

# ---------------------------------------------------------------------------
# Quoting in conditionals
# ---------------------------------------------------------------------------

_TEST_NAME="test: quoted empty string is false for -n"
assert_exit_code 1 $VSH -c 'test -n ""'

_TEST_NAME="test: quoted empty string is true for -z"
assert_exit_code 0 $VSH -c 'test -z ""'

_TEST_NAME="test: quoted string with spaces"
assert_exit_code 0 $VSH -c 'X="hello world"; test -n "$X"'

_TEST_NAME="test: string comparison with quotes"
assert_exit_code 0 $VSH -c '[ "hello" = "hello" ]'

_TEST_NAME="test: string inequality with quotes"
assert_exit_code 1 $VSH -c '[ "hello" = "world" ]'

report
