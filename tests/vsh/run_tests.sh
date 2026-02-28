#!/bin/sh
# run_tests.sh -- Test runner for vsh shell tests
#
# Discovers and runs all test_*.sh files in the same directory.
# Reports aggregate pass/fail/skip counts across all test files.
# Exit code: 0 if all tests pass, 1 if any fail.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VSH="${VSH:-vsh}"
export VSH

# Verify vsh is available
if ! command -v "$VSH" >/dev/null 2>&1; then
    printf "ERROR: '%s' not found in PATH.\n" "$VSH"
    printf "Set VSH=/path/to/vsh or ensure vsh is in PATH.\n"
    exit 2
fi

# Print vsh version info if available
printf "=== vsh Test Suite ===\n"
printf "Shell under test: %s\n" "$VSH"
"$VSH" -c 'echo "vsh ready"' 2>/dev/null || printf "(version info unavailable)\n"
printf "\n"

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0
TOTAL_FILES=0
FAILED_FILES=""

# Discover and run test files
for test_file in "$SCRIPT_DIR"/test_*.sh; do
    [ -f "$test_file" ] || continue

    test_name="$(basename "$test_file")"
    TOTAL_FILES=$((TOTAL_FILES + 1))

    printf "==============================\n"
    printf "Running: %s\n" "$test_name"
    printf "==============================\n"

    # Run the test file in a subshell to isolate state
    if sh "$test_file"; then
        # Parse the last line of output for counts
        :
    else
        :
    fi

    # Capture counts from the test framework output.
    # Each test file sources framework.sh and calls report(), which prints:
    #   --- Results: N passed, N failed, N skipped (total N) ---
    # We re-run in a subshell capturing output to parse these counts.
    _output=$(sh "$test_file" 2>&1) || true
    printf "%s\n" "$_output"

    # Extract counts from the results line
    _results_line=$(printf '%s\n' "$_output" | grep '^--- Results:' | tail -1)
    if [ -n "$_results_line" ]; then
        _p=$(printf '%s' "$_results_line" | sed 's/.*: \([0-9]*\) passed.*/\1/')
        _f=$(printf '%s' "$_results_line" | sed 's/.* \([0-9]*\) failed.*/\1/')
        _s=$(printf '%s' "$_results_line" | sed 's/.* \([0-9]*\) skipped.*/\1/')

        TOTAL_PASS=$((TOTAL_PASS + _p))
        TOTAL_FAIL=$((TOTAL_FAIL + _f))
        TOTAL_SKIP=$((TOTAL_SKIP + _s))

        if [ "$_f" -gt 0 ]; then
            FAILED_FILES="$FAILED_FILES $test_name"
        fi
    else
        printf "  WARNING: Could not parse results from %s\n" "$test_name"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        FAILED_FILES="$FAILED_FILES $test_name"
    fi

    printf "\n"
done

# Final summary
TOTAL_TESTS=$((TOTAL_PASS + TOTAL_FAIL + TOTAL_SKIP))
printf "============================================================\n"
printf "TOTAL: %d files, %d tests: %d passed, %d failed, %d skipped\n" \
    "$TOTAL_FILES" "$TOTAL_TESTS" "$TOTAL_PASS" "$TOTAL_FAIL" "$TOTAL_SKIP"
printf "============================================================\n"

if [ "$TOTAL_FAIL" -gt 0 ]; then
    printf "\nFailed test files:%s\n" "$FAILED_FILES"
    exit 1
fi

if [ "$TOTAL_FILES" -eq 0 ]; then
    printf "\nWARNING: No test files found in %s\n" "$SCRIPT_DIR"
    exit 2
fi

printf "\nAll tests passed.\n"
exit 0
