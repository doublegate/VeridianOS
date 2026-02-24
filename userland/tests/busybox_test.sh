#!/bin/ash
# busybox_test.sh -- Comprehensive BusyBox applet and ash shell test suite
#
# Runs on VeridianOS under ash. Each test prints PASS or FAIL.
# At the end, prints summary and BUSYBOX_ALL_PASS if no failures.

PASS=0
FAIL=0

run_test() {
    NAME="$1"
    EXPECTED="$2"
    ACTUAL="$3"
    if [ "$ACTUAL" = "$EXPECTED" ]; then
        echo "PASS: $NAME"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $NAME (expected '$EXPECTED', got '$ACTUAL')"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== BusyBox Test Suite ==="

# --- Tier 1: Basic applets (no fork required) ---

run_test "echo" "hello" "$(echo hello)"
run_test "pwd" "/" "$(cd /; pwd)"
run_test "true" "0" "$(true; echo $?)"
run_test "false" "1" "$(false; echo $?)"
run_test "basename" "cat_test.txt" "$(basename /usr/src/cat_test.txt)"
run_test "dirname" "/usr/src" "$(dirname /usr/src/cat_test.txt)"
run_test "printf" "hello world" "$(printf '%s %s' hello world)"
run_test "seq" "1 2 3" "$(seq 1 3 | tr '\n' ' ' | sed 's/ $//')"
run_test "yes_head" "y" "$(yes | head -n 1)"
run_test "cat" "CAT_PASS" "$(cat /usr/src/cat_test.txt | tail -n 1)"
run_test "wc_lines" "3" "$(wc -l < /usr/src/wc_test.txt | tr -d ' ')"
run_test "sort_first" "apple" "$(sort /usr/src/sort_test.txt | head -n 1)"
run_test "sort_last" "cherry" "$(sort /usr/src/sort_test.txt | tail -n 1)"
run_test "head" "cherry" "$(head -n 1 /usr/src/sort_test.txt)"
run_test "tail" "banana" "$(tail -n 1 /usr/src/sort_test.txt)"
run_test "uniq" "a b c" "$(printf 'a\na\nb\nc\nc\n' | uniq | tr '\n' ' ' | sed 's/ $//')"
run_test "tr" "HELLO" "$(echo hello | tr a-z A-Z)"
run_test "cut_f1" "hello" "$(echo 'hello:world' | cut -d: -f1)"
run_test "uname_s" "VeridianOS" "$(uname -s)"

# --- Tier 2: test / [ builtin ---

run_test "test_file" "0" "$(test -f /bin/busybox; echo $?)"
run_test "test_dir" "0" "$(test -d /bin; echo $?)"
run_test "test_eq" "0" "$(test 1 -eq 1; echo $?)"
run_test "test_ne" "0" "$(test 1 -ne 2; echo $?)"
run_test "test_str" "0" "$(test -n hello; echo $?)"

# --- Tier 3: Shell features ---

run_test "variable" "hello" "$(X=hello; echo $X)"
run_test "exit_status" "1" "$(false; echo $?)"
run_test "pipe" "3" "$(echo -e 'a\nb\nc' | wc -l | tr -d ' ')"

# Conditional
if test -f /bin/busybox; then
    COND_RESULT="yes"
else
    COND_RESULT="no"
fi
run_test "conditional" "yes" "$COND_RESULT"

# For loop
LOOP_RESULT=""
for i in X Y Z; do
    LOOP_RESULT="${LOOP_RESULT}${i}"
done
run_test "for_loop" "XYZ" "$LOOP_RESULT"

# Arithmetic
run_test "arithmetic" "7" "$(echo $((3 + 4)))"

# --- Summary ---

TOTAL=$((PASS + FAIL))
echo ""
echo "=== Results: $PASS/$TOTAL passed, $FAIL failed ==="

if [ "$FAIL" -eq 0 ]; then
    echo "BUSYBOX_ALL_PASS"
else
    echo "BUSYBOX_SOME_FAIL"
fi
