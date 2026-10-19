#!/usr/bin/env bash
# Lint the KDE cross-pipeline files changed or added in the working tree:
# bash -n and shellcheck (-S warning) on shell scripts, ruff check, ruff
# format --check and py_compile on Python, and check that
# tools/cross/PATCHES.md matches what tools/cross/list-patches.py generates.
#
# Usage: scripts/dev/lint-cross.sh
set -u
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 2
cd "$root" || exit 2
mapfile -t all < <({ git diff --name-only -- tools/cross; git ls-files --others --exclude-standard tools/cross; } | sort -u)
sh=() py=(tools/cross/list-patches.py)
for f in "${all[@]}"; do
    [ -f "$f" ] || continue
    case $f in
        *.sh) sh+=("$f") ;;
        *.py) py+=("$f") ;;
    esac
done
rc=0
for f in "${sh[@]}"; do bash -n "$f" || { echo "SYNTAX $f"; rc=1; }; done
echo "bash -n: ${#sh[@]} files"
if [ ${#sh[@]} -gt 0 ]; then
    if command -v shellcheck >/dev/null; then
        shellcheck -x -S warning "${sh[@]}" || rc=1
    else
        echo "shellcheck: not installed"
    fi
fi
ruff check "${py[@]}" || rc=1
ruff format --check "${py[@]}" || rc=1
for f in "${py[@]}"; do python3 -m py_compile "$f" || { echo "PYCOMPILE $f"; rc=1; }; done
before="$(mktemp)"
cp tools/cross/PATCHES.md "$before"
python3 tools/cross/list-patches.py >/dev/null || rc=1
if ! cmp -s tools/cross/PATCHES.md "$before"; then
    echo "PATCHES.md was out of date (regenerated now; review and stage it)"
    rc=1
fi
rm -f "$before"
echo "=== LINT rc=$rc"
exit $rc
