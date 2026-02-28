#!/bin/sh
# framework.sh -- Test framework for vsh shell tests
#
# Provides assertion helpers, counters, and summary reporting.
# Source this file at the top of each test_*.sh script.

VSH="${VSH:-vsh}"
_PASS_COUNT=0
_FAIL_COUNT=0
_SKIP_COUNT=0
_TEST_NAME=""

pass() {
    _PASS_COUNT=$((_PASS_COUNT + 1))
    printf "  PASS: %s\n" "$_TEST_NAME"
}

fail() {
    _FAIL_COUNT=$((_FAIL_COUNT + 1))
    printf "  FAIL: %s -- %s\n" "$_TEST_NAME" "$1"
}

skip() {
    _SKIP_COUNT=$((_SKIP_COUNT + 1))
    printf "  SKIP: %s -- %s\n" "$_TEST_NAME" "$1"
}

# assert_equal ACTUAL EXPECTED [MESSAGE]
assert_equal() {
    if [ "$1" = "$2" ]; then
        pass
    else
        fail "${3:-expected '$2', got '$1'}"
    fi
}

# assert_not_equal ACTUAL UNEXPECTED [MESSAGE]
assert_not_equal() {
    if [ "$1" != "$2" ]; then
        pass
    else
        fail "${3:-did not expect '$2'}"
    fi
}

# assert_match ACTUAL PATTERN [MESSAGE]
# Uses grep -qE for extended regex matching.
assert_match() {
    if printf '%s' "$1" | grep -qE "$2" 2>/dev/null; then
        pass
    else
        fail "${3:-'$1' does not match pattern '$2'}"
    fi
}

# assert_exit_code EXPECTED_CODE COMMAND...
# Runs the command and checks its exit code.
assert_exit_code() {
    _expected_code="$1"
    shift
    "$@" >/dev/null 2>&1
    _actual_code=$?
    if [ "$_actual_code" -eq "$_expected_code" ]; then
        pass
    else
        fail "expected exit code $_expected_code, got $_actual_code"
    fi
}

# assert_contains HAYSTACK NEEDLE [MESSAGE]
assert_contains() {
    case "$1" in
        *"$2"*) pass ;;
        *)      fail "${3:-'$1' does not contain '$2'}" ;;
    esac
}

# report -- Print final summary. Returns 0 if all passed, 1 otherwise.
report() {
    _total=$((_PASS_COUNT + _FAIL_COUNT + _SKIP_COUNT))
    printf "\n--- Results: %d passed, %d failed, %d skipped (total %d) ---\n" \
        "$_PASS_COUNT" "$_FAIL_COUNT" "$_SKIP_COUNT" "$_total"
    if [ "$_FAIL_COUNT" -gt 0 ]; then
        return 1
    fi
    return 0
}
