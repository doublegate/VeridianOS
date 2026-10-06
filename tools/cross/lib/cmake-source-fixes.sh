# Shared source fixes for the KDE cross-build scripts (sourced, not run).
# The sourcing script must define die().

# Many frameworks require the Qt Test component at top level although only
# their autotests use it; BUILD_TESTING is off and Qt is built with
# -no-feature-testlib. Drop "Test" from multi-component Qt6 requirements and
# make a standalone Qt6Test lookup optional.
relax_qt_test() {
    local f="$1"
    python3 - "${f}" <<'PYEOF' || die "failed to relax Qt Test requirement in ${f}"
import re, sys
p = sys.argv[1]
s = open(p).read()
KEYWORDS = {"REQUIRED", "COMPONENTS", "OPTIONAL_COMPONENTS", "CONFIG", "NO_MODULE"}
def fix(m):
    # One whole find_package(Qt6 ...) call, single- or multi-line.
    block = m.group(0)
    toks = block[len("find_package("):-1].split()
    if "REQUIRED" not in toks or "Test" not in toks:
        return block
    comps = [t for t in toks[toks.index("REQUIRED") + 1:]
             if t not in KEYWORDS and not t.startswith("$")]
    if comps == ["Test"]:
        return block  # Test is the only component: a test-only lookup
    return re.sub(r"\s+Test(?=[\s)])", "", block)
s2 = re.sub(r"find_package\(Qt6\s[^)]*\)", fix, s)
s2 = re.sub(r"(find_package\(Qt6Test\b[^)\n]*?)\s+REQUIRED(\s*\))", r"\1\2", s2)
if s2 != s:
    open(p, "w").write(s2)
PYEOF
}

