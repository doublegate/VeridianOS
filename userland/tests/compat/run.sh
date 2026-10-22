#!/bin/sh
# In-guest runner for the conformance suites (LTP syscalls, the Open POSIX
# Test Suite, musl libc-test) on the compat root filesystem
# (scripts/build-compat-rootfs.sh). scripts/run-compat-tests.sh starts it at
# the shell prompt and reads its output from the serial console.
#
#   /bin/ash /usr/lib/compat/run.sh LIST [FIRST [SECONDS [LAST]]]
#
# LIST (tools/compat/compat.py lists) has one test per line:
#
#   N ID DIR PROGRAM [ARG...]
#
# N numbers the lines, DIR is where the test runs (`-` for /tmp), PROGRAM is
# its path in the image, or `-` when it was not built. FIRST is the N to
# start at (1 by default): when a test takes the whole system down, the host
# boots again and carries on after it. SECONDS is the limit for each test
# (default 60). LAST, when given, is the N to stop after (the host can split
# a suite between several guests).
#
# Output, one line each:
#   COMPAT-BEGIN N ID
#   COMPAT-LOG N TEXT        a few lines of a test's output when it does not
#                            exit 0
#   COMPAT-END N ID RESULT   RESULT is "rc=.. sig=.. timeout=.. ms=..", or
#                            "notbuilt"
#   COMPAT-SUMMARY LIST N    after the last line
# compat_exec prints the LOG and END lines.
#
# Everything here but compat_exec is a shell builtin, so each test costs one
# process (plus the test's own): creating processes is slow on VeridianOS,
# and a few per test cost more than the tests. The host turns RESULT into
# PASS/FAIL/... per suite; nothing here knows the suites' conventions.

list="$1"
first="${2:-1}"
limit="${3:-60}"
stop="${4:-}"
# The environment can point elsewhere (the host-side tests do).
exec_helper="${COMPAT_EXEC:-/usr/lib/compat/compat_exec}"
out="${COMPAT_OUT:-/tmp/compat.out}"

export LTPROOT=/opt/ltp
export PATH="/opt/ltp/testcases/bin:$PATH"
export TMPDIR=/tmp
export LTP_COLORIZE_OUTPUT=0

# ARGs are split into words but never matched against files.
set -f

if [ ! -r "$list" ]; then
    echo "COMPAT-ERROR no list $list"
    exit 2
fi

# sed skips to FIRST: ash's `read` takes a byte per system call, and
# reading past a thousand long lines took minutes under emulation. The loop
# runs in a subshell (the pipeline), so it prints the summary itself;
# `read` empties n at the end, so the count is kept apart.
sed -n "$first,\$p" "$list" | {
    last=0
    while read -r n id dir prog args; do
        [ -z "$id" ] && continue
        [ -n "$stop" ] && [ "$n" -gt "$stop" ] && break
        last="$n"
        echo "COMPAT-BEGIN $n $id"
        if [ "$prog" = - ] || [ ! -f "$prog" ]; then
            echo "COMPAT-END $n $id notbuilt"
            continue
        fi
        # compat_exec also removes what a failed test left in /tmp. It exits 0,
        # or 3 when the limit killed the test; anything else means it died
        # before reporting, and the runner stops so that the host records
        # this test (the prompt comes back during it) and carries on after.
        # shellcheck disable=SC2086 # ARGs are split into words on purpose
        "$exec_helper" "$limit" "$out" "$n" "$id" "$dir" "$prog" $args </dev/null
        rc=$?
        [ "$rc" -eq 0 ] || [ "$rc" -eq 3 ] || exit 1
    done
    echo "COMPAT-SUMMARY $list $last"
}
