#!/usr/bin/env bash
# pkg-cc.sh SRC BLD FILE REL
#   Compile FILE (an edited copy of SRC/REL) with the exact flags the package's
#   build used for SRC/REL, syntax-only, and print the compiler's diagnostics.
#   SRC: the package's pristine source tree; BLD: its build tree.
#   Works for CMake Makefile builds (flags.make), Meson/Ninja and CMake/Ninja
#   builds (compile_commands.json) and Automake builds (make -n in BLD).
#   Exit status: the compiler's (2: no compile command found for REL).
set -uo pipefail
SRC=${1%/}
BLD=${2%/}
FILE=$3
REL=$4
ABS="${SRC}/${REL}"

run() { # dir, command (with the original source path in it)
    local dir=$1 cmd=$2
    # Syntax only; the edited file replaces the original source.
    cmd=${cmd//"${ABS}"/"${FILE}"}
    cmd=$(printf '%s' "$cmd" | sed -E 's/ -o [^ ]+/ /; s/ -c / /; s/ -M[DT]? [^ ]+//g; s/ -MF [^ ]+//g; s/ -MQ [^ ]+//g; s/ -MMD//g; s/ -MD//g')
    # Includes relative to the original file's directory still resolve.
    (cd "$dir" && eval "$cmd -I$(dirname "$ABS") -fsyntax-only")
}

# 1. compile_commands.json (Meson, CMake with export).
cc_json=$(find "$BLD" -maxdepth 2 -name compile_commands.json | head -1)
if [[ -n "$cc_json" ]]; then
    out=$(python3 -I - "$cc_json" "$ABS" <<'PY'
import json, shlex, sys, os
db, target = sys.argv[1], os.path.realpath(sys.argv[2])
for e in json.load(open(db)):
    f = e["file"] if os.path.isabs(e["file"]) else os.path.join(e["directory"], e["file"])
    if os.path.realpath(f) == target:
        cmd = e.get("command") or " ".join(shlex.quote(a) for a in e["arguments"])
        print(e["directory"]); print(cmd.replace(e["file"], target) if not os.path.isabs(e["file"]) else cmd)
        break
PY
)
    if [[ -n "$out" ]]; then
        run "$(head -1 <<<"$out")" "$(tail -n +2 <<<"$out")"
        exit $?
    fi
fi

# 2. CMake Makefiles: the target whose build.make compiles REL.
mapfile -t bms < <(find "$BLD" -name build.make 2>/dev/null)
[[ ${#bms[@]} -gt 0 ]] && mapfile -t bms < <(grep -l -F -e "${ABS}" "${bms[@]}" 2>/dev/null)
for bm in "${bms[@]}"; do
    fm="$(dirname "$bm")/flags.make"
    [[ -f "$fm" ]] && grep -q -F -e "-c ${ABS}" "$bm" || continue
    lang=CXX; [[ "$REL" == *.c ]] && lang=C
    compiler=$(grep -m1 -F -e "-c ${ABS}" "$bm" | grep -o '/[^ ]*\(g++\|gcc\|c++\|cc\)' | head -1)
    wd=$(grep -m1 -F -e "-c ${ABS}" "$bm" | grep -o 'cd [^ ]*' | head -1 | cut -c4-)
    cmd="$compiler $(sed -n "s/^${lang}_DEFINES = //p" "$fm") $(sed -n "s/^${lang}_INCLUDES = //p" "$fm") $(sed -n "s/^${lang}_FLAGS = //p" "$fm") -c ${ABS}"
    run "${wd:-$(dirname "$bm")}" "$cmd"
    exit $?
done

# 3. Automake: ask make (dry run, as if only the source changed) for the
#    command that compiles it.
dir="$BLD/$(dirname "$REL")"
base=$(basename "$REL"); stem=${base%.*}
if [[ -f "$dir/Makefile" ]]; then
    for obj in $(grep -o "[A-Za-z0-9_-]*${stem}\.l\?o\b" "$dir/Makefile" | sort -u); do
        cmd=$(make -C "$dir" -n V=1 -W "$base" "$obj" 2>/dev/null | grep -E -- "-c .*${base}|${base}.*-c " | tail -1)
        [[ -n "$cmd" ]] || continue
        cmd=${cmd##*--mode=compile }
        cmd=${cmd##*; }
        # The source as automake names it: relative, maybe through $(srcdir).
        cmd=$(printf '%s' "$cmd" | sed -E "s#\`test -f '[^']*' \|\| echo '[^']*'\`${base}# ${FILE}#; s#(^| )(\./)?${base}( |\$)# ${FILE} #")
        run "$dir" "$cmd"
        exit $?
    done
fi
echo "pkg-cc: no compile command found for ${REL}" >&2
exit 2
