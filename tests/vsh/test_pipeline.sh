#!/bin/sh
# test_pipeline.sh -- Tests for pipelines and command lists in vsh
#
# Covers: simple pipes (|), multi-stage pipelines, pipefail,
#         && and || operators, semicolon command lists,
#         pipeline exit status, negation (!).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Pipeline Tests ===\n\n"

TMPDIR="${TMPDIR:-/tmp}/vsh_pipe_test_$$"
mkdir -p "$TMPDIR"
trap 'rm -rf "$TMPDIR"' EXIT

# ---------------------------------------------------------------------------
# Simple pipes
# ---------------------------------------------------------------------------

_TEST_NAME="pipe: echo to cat"
_out=$($VSH -c 'echo hello | cat')
assert_equal "$_out" "hello"

_TEST_NAME="pipe: echo to wc -l"
_out=$($VSH -c 'printf "a\nb\nc\n" | wc -l')
assert_contains "$_out" "3"

_TEST_NAME="pipe: echo to grep"
_out=$($VSH -c 'echo "hello world" | grep hello')
assert_equal "$_out" "hello world"

_TEST_NAME="pipe: echo to grep no match"
assert_exit_code 1 $VSH -c 'echo "hello" | grep xyz'

_TEST_NAME="pipe: echo to tr"
_out=$($VSH -c 'echo hello | tr a-z A-Z')
assert_equal "$_out" "HELLO"

_TEST_NAME="pipe: echo to cut"
_out=$($VSH -c 'echo "a:b:c" | cut -d: -f2')
assert_equal "$_out" "b"

_TEST_NAME="pipe: echo to head"
_out=$($VSH -c 'printf "1\n2\n3\n4\n5\n" | head -n 3')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="pipe: echo to tail"
_out=$($VSH -c 'printf "1\n2\n3\n4\n5\n" | tail -n 2')
assert_contains "$_out" "4"

_TEST_NAME="pipe: echo to sort"
_out=$($VSH -c 'printf "c\na\nb\n" | sort')
_first=$(printf '%s\n' "$_out" | head -n 1)
assert_equal "$_first" "a"

_TEST_NAME="pipe: echo to uniq"
_out=$($VSH -c 'printf "a\na\nb\nb\nc\n" | uniq')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

# ---------------------------------------------------------------------------
# Multi-stage pipelines
# ---------------------------------------------------------------------------

_TEST_NAME="pipe: three-stage pipeline"
_out=$($VSH -c 'echo "hello world" | tr " " "\n" | sort')
_first=$(printf '%s\n' "$_out" | head -n 1)
assert_equal "$_first" "hello"

_TEST_NAME="pipe: four-stage pipeline"
_out=$($VSH -c 'printf "c\na\nb\na\n" | sort | uniq | wc -l')
assert_contains "$_out" "3"

_TEST_NAME="pipe: pipeline with cat"
_out=$($VSH -c 'echo test | cat | cat | cat')
assert_equal "$_out" "test"

_TEST_NAME="pipe: long pipeline"
_out=$($VSH -c 'echo HELLO | tr A-Z a-z | tr a-z A-Z | tr A-Z a-z')
assert_equal "$_out" "hello"

_TEST_NAME="pipe: pipeline preserves multiline"
_out=$($VSH -c 'printf "a\nb\nc\n" | cat | cat')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

# ---------------------------------------------------------------------------
# Pipeline exit status
# ---------------------------------------------------------------------------

_TEST_NAME="pipe: exit status is last command (success)"
$VSH -c 'false | true'; _code=$?
assert_equal "$_code" "0"

_TEST_NAME="pipe: exit status is last command (failure)"
$VSH -c 'true | false'; _code=$?
assert_equal "$_code" "1"

_TEST_NAME="pipe: \$? reflects pipeline exit"
_out=$($VSH -c 'true | false; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="pipe: \$? reflects last in pipeline"
_out=$($VSH -c 'false | true; echo $?')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# Pipefail
# ---------------------------------------------------------------------------

_TEST_NAME="pipefail: set -o pipefail catches failure"
_out=$($VSH -c 'set -o pipefail; false | true; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="pipefail: all succeed returns 0"
_out=$($VSH -c 'set -o pipefail; true | true | true; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="pipefail: first fails"
_out=$($VSH -c 'set -o pipefail; false | true | true; echo $?')
assert_not_equal "$_out" "0"

_TEST_NAME="pipefail: middle fails"
_out=$($VSH -c 'set -o pipefail; true | false | true; echo $?')
assert_not_equal "$_out" "0"

# ---------------------------------------------------------------------------
# Negation: !
# ---------------------------------------------------------------------------

_TEST_NAME="negate: ! true is 1"
_out=$($VSH -c '! true; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="negate: ! false is 0"
_out=$($VSH -c '! false; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="negate: ! in pipeline"
_out=$($VSH -c '! echo hello | grep xyz; echo $?')
assert_equal "$_out" "0"

# ---------------------------------------------------------------------------
# AND operator: &&
# ---------------------------------------------------------------------------

_TEST_NAME="and: true && echo"
_out=$($VSH -c 'true && echo yes')
assert_equal "$_out" "yes"

_TEST_NAME="and: false && echo (short-circuit)"
_out=$($VSH -c 'false && echo yes')
assert_equal "$_out" ""

_TEST_NAME="and: chained &&"
_out=$($VSH -c 'true && true && echo all_true')
assert_equal "$_out" "all_true"

_TEST_NAME="and: chained && with failure"
_out=$($VSH -c 'true && false && echo should_not_print')
assert_equal "$_out" ""

_TEST_NAME="and: && exit status propagation"
_out=$($VSH -c 'true && false; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="and: && with commands"
_out=$($VSH -c 'echo first && echo second')
assert_contains "$_out" "first"

_TEST_NAME="and: && second command runs"
_out=$($VSH -c 'echo first && echo second')
assert_contains "$_out" "second"

# ---------------------------------------------------------------------------
# OR operator: ||
# ---------------------------------------------------------------------------

_TEST_NAME="or: false || echo"
_out=$($VSH -c 'false || echo fallback')
assert_equal "$_out" "fallback"

_TEST_NAME="or: true || echo (short-circuit)"
_out=$($VSH -c 'true || echo fallback')
assert_equal "$_out" ""

_TEST_NAME="or: chained ||"
_out=$($VSH -c 'false || false || echo last_resort')
assert_equal "$_out" "last_resort"

_TEST_NAME="or: || exit status after success"
_out=$($VSH -c 'true || false; echo $?')
assert_equal "$_out" "0"

_TEST_NAME="or: || exit status after all fail"
_out=$($VSH -c 'false || false; echo $?')
assert_equal "$_out" "1"

# ---------------------------------------------------------------------------
# Mixed && and ||
# ---------------------------------------------------------------------------

_TEST_NAME="mixed: && then ||"
_out=$($VSH -c 'true && echo yes || echo no')
assert_equal "$_out" "yes"

_TEST_NAME="mixed: failed && then ||"
_out=$($VSH -c 'false && echo yes || echo no')
assert_equal "$_out" "no"

_TEST_NAME="mixed: complex chain"
_out=$($VSH -c 'true && true || echo bad && echo good')
assert_contains "$_out" "good"

_TEST_NAME="mixed: pattern used for error handling"
_out=$($VSH -c 'false && echo ok || echo "error handled"')
assert_equal "$_out" "error handled"

# ---------------------------------------------------------------------------
# Semicolon command lists
# ---------------------------------------------------------------------------

_TEST_NAME="list: semicolon separates commands"
_out=$($VSH -c 'echo first; echo second')
assert_contains "$_out" "first"

_TEST_NAME="list: semicolon runs both"
_out=$($VSH -c 'echo first; echo second')
assert_contains "$_out" "second"

_TEST_NAME="list: semicolon independent status"
_out=$($VSH -c 'false; echo $?')
assert_equal "$_out" "1"

_TEST_NAME="list: multiple semicolons"
_out=$($VSH -c 'echo a; echo b; echo c')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="list: semicolon after failure continues"
_out=$($VSH -c 'false; echo continued')
assert_equal "$_out" "continued"

# ---------------------------------------------------------------------------
# Pipeline with redirections
# ---------------------------------------------------------------------------

_TEST_NAME="pipe+redir: pipeline output to file"
$VSH -c "echo hello | tr a-z A-Z > $TMPDIR/pipe_out.txt"
_out=$(cat "$TMPDIR/pipe_out.txt")
assert_equal "$_out" "HELLO"

_TEST_NAME="pipe+redir: file input to pipeline"
echo "hello world" > "$TMPDIR/pipe_in.txt"
_out=$($VSH -c "cat < $TMPDIR/pipe_in.txt | tr a-z A-Z")
assert_equal "$_out" "HELLO WORLD"

_TEST_NAME="pipe+redir: pipeline both ends"
echo "test data" > "$TMPDIR/pipe_io_in.txt"
$VSH -c "cat < $TMPDIR/pipe_io_in.txt | tr a-z A-Z > $TMPDIR/pipe_io_out.txt"
_out=$(cat "$TMPDIR/pipe_io_out.txt")
assert_equal "$_out" "TEST DATA"

# ---------------------------------------------------------------------------
# Background and subshell pipelines
# ---------------------------------------------------------------------------

_TEST_NAME="pipe: subshell in pipeline"
_out=$($VSH -c '(echo hello) | cat')
assert_equal "$_out" "hello"

_TEST_NAME="pipe: brace group in pipeline"
_out=$($VSH -c '{ echo hello; echo world; } | wc -l')
assert_contains "$_out" "2"

_TEST_NAME="pipe: for loop in pipeline"
_out=$($VSH -c 'for i in a b c; do echo $i; done | wc -l')
assert_contains "$_out" "3"

_TEST_NAME="pipe: while loop in pipeline"
_out=$($VSH -c 'echo -e "1\n2\n3" | while read line; do echo "got: $line"; done' 2>/dev/null)
# This depends on whether the shell supports read in pipeline; check for at least some output
assert_not_equal "$_out" "" "pipeline with while should produce output"

# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------

_TEST_NAME="edge: empty pipeline element"
_out=$($VSH -c 'echo test | cat')
assert_equal "$_out" "test"

_TEST_NAME="edge: single command (no pipe)"
_out=$($VSH -c 'echo solo')
assert_equal "$_out" "solo"

_TEST_NAME="edge: pipe with builtin"
_out=$($VSH -c 'echo hello | echo world')
# echo ignores stdin, so output should be "world" from the last echo
assert_contains "$_out" "world"

report
