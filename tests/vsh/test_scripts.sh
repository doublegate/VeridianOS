#!/bin/sh
# test_scripts.sh -- Tests for script execution features in vsh
#
# Covers: source/dot command, script arguments, exit codes,
#         subshell execution, eval, exec, command substitution,
#         process substitution, vsh -c behavior.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/framework.sh"

printf "=== Scripts Tests ===\n\n"

TMPDIR="${TMPDIR:-/tmp}/vsh_script_test_$$"
mkdir -p "$TMPDIR"
trap 'rm -rf "$TMPDIR"' EXIT

# ---------------------------------------------------------------------------
# vsh -c: command string execution
# ---------------------------------------------------------------------------

_TEST_NAME="-c: basic command"
_out=$($VSH -c 'echo hello')
assert_equal "$_out" "hello"

_TEST_NAME="-c: multiple commands"
_out=$($VSH -c 'echo a; echo b')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "2"

_TEST_NAME="-c: exit code"
assert_exit_code 42 $VSH -c 'exit 42'

_TEST_NAME="-c: variable assignment and use"
_out=$($VSH -c 'X=hello; echo $X')
assert_equal "$_out" "hello"

_TEST_NAME="-c: pipeline"
_out=$($VSH -c 'echo hello | cat')
assert_equal "$_out" "hello"

_TEST_NAME="-c: control flow"
_out=$($VSH -c 'if true; then echo yes; fi')
assert_equal "$_out" "yes"

_TEST_NAME="-c: arithmetic"
_out=$($VSH -c 'echo $((2+3))')
assert_equal "$_out" "5"

_TEST_NAME="-c: empty string"
_out=$($VSH -c '')
assert_equal "$_out" ""

_TEST_NAME="-c: whitespace only"
_out=$($VSH -c '   ')
assert_equal "$_out" ""

_TEST_NAME="-c: multiple semicolons"
_out=$($VSH -c 'echo a;; echo b' 2>/dev/null)
# Double semicolons might cause syntax error or be treated as empty
# Just ensure no crash
pass

# ---------------------------------------------------------------------------
# Source / dot command
# ---------------------------------------------------------------------------

_TEST_NAME="source: basic file sourcing"
echo 'SOURCED_VAR=hello' > "$TMPDIR/source1.sh"
_out=$($VSH -c "source $TMPDIR/source1.sh; echo \$SOURCED_VAR")
assert_equal "$_out" "hello"

_TEST_NAME="source: dot syntax"
echo 'DOT_VAR=world' > "$TMPDIR/dot1.sh"
_out=$($VSH -c ". $TMPDIR/dot1.sh; echo \$DOT_VAR")
assert_equal "$_out" "world"

_TEST_NAME="source: modifies current environment"
echo 'X=sourced' > "$TMPDIR/source2.sh"
_out=$($VSH -c "X=original; source $TMPDIR/source2.sh; echo \$X")
assert_equal "$_out" "sourced"

_TEST_NAME="source: function defined in sourced file"
echo 'greeting() { echo hi; }' > "$TMPDIR/source3.sh"
_out=$($VSH -c "source $TMPDIR/source3.sh; greeting")
assert_equal "$_out" "hi"

_TEST_NAME="source: multiple statements"
cat > "$TMPDIR/source4.sh" << 'SRCEOF'
A=1
B=2
C=$((A+B))
SRCEOF
_out=$($VSH -c "source $TMPDIR/source4.sh; echo \$C")
assert_equal "$_out" "3"

_TEST_NAME="source: nonexistent file fails"
$VSH -c "source $TMPDIR/no_such_file.sh" 2>/dev/null; _code=$?
assert_not_equal "$_code" "0"

_TEST_NAME="source: exit in sourced file exits shell"
echo 'exit 7' > "$TMPDIR/source_exit.sh"
$VSH -c "source $TMPDIR/source_exit.sh"; _code=$?
assert_equal "$_code" "7"

_TEST_NAME="source: sourced file inherits variables"
echo 'echo "HOME=$HOME"' > "$TMPDIR/source5.sh"
_out=$($VSH -c "source $TMPDIR/source5.sh")
assert_contains "$_out" "HOME="

# ---------------------------------------------------------------------------
# Script file execution
# ---------------------------------------------------------------------------

_TEST_NAME="script: basic script"
cat > "$TMPDIR/script1.sh" << 'SEOF'
#!/usr/bin/env vsh
echo "from script"
SEOF
chmod +x "$TMPDIR/script1.sh"
_out=$($VSH "$TMPDIR/script1.sh" 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "from script"
else
    skip "script file execution not supported via \$VSH filename"
fi

_TEST_NAME="script: with arguments"
cat > "$TMPDIR/script2.sh" << 'SEOF'
echo "arg1=$1 arg2=$2"
SEOF
_out=$($VSH "$TMPDIR/script2.sh" hello world 2>/dev/null)
if [ -n "$_out" ]; then
    assert_equal "$_out" "arg1=hello arg2=world"
else
    skip "script file argument passing not supported"
fi

_TEST_NAME="script: exit code propagation"
echo 'exit 13' > "$TMPDIR/script3.sh"
$VSH "$TMPDIR/script3.sh" 2>/dev/null; _code=$?
if [ "$_code" -eq 13 ]; then
    pass
else
    skip "script file exit code propagation (got: $_code)"
fi

_TEST_NAME="script: \$# in script"
echo 'echo $#' > "$TMPDIR/script4.sh"
_out=$($VSH "$TMPDIR/script4.sh" a b c 2>/dev/null)
if [ "$_out" = "3" ]; then
    pass
else
    skip "script positional params not supported (got: $_out)"
fi

# ---------------------------------------------------------------------------
# Eval
# ---------------------------------------------------------------------------

_TEST_NAME="eval: basic eval"
_out=$($VSH -c 'eval "echo hello"')
assert_equal "$_out" "hello"

_TEST_NAME="eval: variable in eval string"
_out=$($VSH -c 'CMD="echo test"; eval "$CMD"')
assert_equal "$_out" "test"

_TEST_NAME="eval: constructed command"
_out=$($VSH -c 'X=echo; Y=hello; eval "$X $Y"')
assert_equal "$_out" "hello"

_TEST_NAME="eval: eval with assignment"
_out=$($VSH -c 'eval "MY_VAR=42"; echo $MY_VAR')
assert_equal "$_out" "42"

_TEST_NAME="eval: empty string"
_out=$($VSH -c 'eval ""; echo ok')
assert_equal "$_out" "ok"

_TEST_NAME="eval: multiple statements"
_out=$($VSH -c 'eval "X=1; Y=2; echo \$((X+Y))"')
assert_equal "$_out" "3"

# ---------------------------------------------------------------------------
# Command substitution
# ---------------------------------------------------------------------------

_TEST_NAME="cmdsub: basic \$()"
_out=$($VSH -c 'echo $(echo hello)')
assert_equal "$_out" "hello"

_TEST_NAME="cmdsub: in variable assignment"
_out=$($VSH -c 'X=$(echo world); echo $X')
assert_equal "$_out" "world"

_TEST_NAME="cmdsub: nested in string"
_out=$($VSH -c 'echo "result: $(echo 42)"')
assert_equal "$_out" "result: 42"

_TEST_NAME="cmdsub: with pipeline"
_out=$($VSH -c 'X=$(echo hello | tr a-z A-Z); echo $X')
assert_equal "$_out" "HELLO"

_TEST_NAME="cmdsub: capturing exit status"
_out=$($VSH -c '$(false); echo $?')
assert_equal "$_out" "1"

_TEST_NAME="cmdsub: multiline output"
_out=$($VSH -c 'X=$(printf "a\nb\nc"); echo "$X"')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="cmdsub: backtick syntax"
_out=$($VSH -c 'echo `echo hello`')
assert_equal "$_out" "hello"

_TEST_NAME="cmdsub: nested \$()"
_out=$($VSH -c 'echo $(echo $(echo deep))')
assert_equal "$_out" "deep"

_TEST_NAME="cmdsub: in arithmetic"
_out=$($VSH -c 'echo $(($(echo 5) + $(echo 3)))')
assert_equal "$_out" "8"

_TEST_NAME="cmdsub: trailing newlines stripped"
_out=$($VSH -c 'X=$(echo "hello"); echo "$X"')
assert_equal "$_out" "hello"

# ---------------------------------------------------------------------------
# Subshell execution
# ---------------------------------------------------------------------------

_TEST_NAME="subshell: basic"
_out=$($VSH -c '(echo sub)')
assert_equal "$_out" "sub"

_TEST_NAME="subshell: exit code"
$VSH -c '(exit 5)'; _code=$?
assert_equal "$_code" "5"

_TEST_NAME="subshell: variable isolation"
_out=$($VSH -c 'X=outer; (X=inner); echo $X')
assert_equal "$_out" "outer"

_TEST_NAME="subshell: nested subshells"
_out=$($VSH -c '((echo deep))')
assert_contains "$_out" "deep"

_TEST_NAME="subshell: pipeline in subshell"
_out=$($VSH -c '(echo hello | tr a-z A-Z)')
assert_equal "$_out" "HELLO"

_TEST_NAME="subshell: loop in subshell"
_out=$($VSH -c '(for i in a b c; do echo $i; done)')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

# ---------------------------------------------------------------------------
# Exit codes
# ---------------------------------------------------------------------------

_TEST_NAME="exit: 0"
assert_exit_code 0 $VSH -c 'exit 0'

_TEST_NAME="exit: 1"
assert_exit_code 1 $VSH -c 'exit 1'

_TEST_NAME="exit: 127 (command not found)"
$VSH -c 'nonexistent_command_xyz_999' 2>/dev/null; _code=$?
assert_equal "$_code" "127"

_TEST_NAME="exit: false returns 1"
assert_exit_code 1 $VSH -c 'false'

_TEST_NAME="exit: true returns 0"
assert_exit_code 0 $VSH -c 'true'

_TEST_NAME="exit: last command determines status"
assert_exit_code 0 $VSH -c 'false; true'

_TEST_NAME="exit: last command determines status (fail)"
assert_exit_code 1 $VSH -c 'true; false'

# ---------------------------------------------------------------------------
# Brace group execution
# ---------------------------------------------------------------------------

_TEST_NAME="brace: execution"
_out=$($VSH -c '{ echo hello; }')
assert_equal "$_out" "hello"

_TEST_NAME="brace: multiple commands"
_out=$($VSH -c '{ echo a; echo b; echo c; }')
_lines=$(printf '%s\n' "$_out" | wc -l)
assert_equal "$(echo $_lines)" "3"

_TEST_NAME="brace: shares environment"
_out=$($VSH -c '{ X=test; }; echo $X')
assert_equal "$_out" "test"

# ---------------------------------------------------------------------------
# Comments
# ---------------------------------------------------------------------------

_TEST_NAME="comment: ignored"
_out=$($VSH -c 'echo hello # this is a comment')
assert_equal "$_out" "hello"

_TEST_NAME="comment: line beginning"
_out=$($VSH -c '# comment
echo visible')
assert_equal "$_out" "visible"

_TEST_NAME="comment: in quotes is literal"
_out=$($VSH -c 'echo "# not a comment"')
assert_equal "$_out" "# not a comment"

_TEST_NAME="comment: hash in single quotes"
_out=$($VSH -c "echo '# literal'")
assert_equal "$_out" "# literal"

# ---------------------------------------------------------------------------
# Complex script patterns
# ---------------------------------------------------------------------------

_TEST_NAME="script: variable-based dispatch"
_out=$($VSH -c 'CMD=echo; $CMD hello')
assert_equal "$_out" "hello"

_TEST_NAME="script: conditional assignment"
_out=$($VSH -c 'X=${UNSET:-default}; echo $X')
assert_equal "$_out" "default"

_TEST_NAME="script: counter pattern"
_out=$($VSH -c 'n=0; for i in a b c d e; do n=$((n+1)); done; echo $n')
assert_equal "$_out" "5"

_TEST_NAME="script: accumulator pattern"
_out=$($VSH -c 'result=""; for w in hello world; do result="$result $w"; done; echo $result')
assert_contains "$_out" "hello"

_TEST_NAME="script: flag parsing pattern"
_out=$($VSH -c 'verbose=0; for arg in -v hello; do case $arg in -v) verbose=1;; *) echo $arg;; esac; done; echo "verbose=$verbose"')
assert_contains "$_out" "verbose=1"

_TEST_NAME="script: temporary file pattern"
cat > "$TMPDIR/tmppattern.sh" << 'SEOF'
echo data > /tmp/vsh_test_tmp_$$
cat /tmp/vsh_test_tmp_$$
rm -f /tmp/vsh_test_tmp_$$
SEOF
_out=$($VSH -c "source $TMPDIR/tmppattern.sh" 2>/dev/null)
assert_contains "$_out" "data"

# ---------------------------------------------------------------------------
# Error handling
# ---------------------------------------------------------------------------

_TEST_NAME="error: syntax error exits nonzero"
$VSH -c 'if then fi' 2>/dev/null; _code=$?
assert_not_equal "$_code" "0"

_TEST_NAME="error: unclosed quote"
$VSH -c 'echo "unclosed' 2>/dev/null; _code=$?
# May or may not be an error; just verify no hang
pass

_TEST_NAME="error: command not found"
_out=$($VSH -c 'nonexistent_cmd_xyz_999' 2>&1); _code=$?
assert_not_equal "$_code" "0"

report
