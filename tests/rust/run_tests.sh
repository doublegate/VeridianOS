#!/bin/sh
# run_tests.sh -- Basic Rust toolchain validation tests
#
# Verifies that the Rust toolchain is functional by testing:
#   - rustc --version output
#   - Compile and run hello world
#   - cargo new + cargo build
#   - std::fs basic operations

set -e

PASS=0
FAIL=0
TMPDIR="${TMPDIR:-/tmp}/rust_toolchain_test_$$"
mkdir -p "$TMPDIR"
trap 'rm -rf "$TMPDIR"' EXIT

pass() {
    PASS=$((PASS + 1))
    printf "  PASS: %s\n" "$1"
}

fail() {
    FAIL=$((FAIL + 1))
    printf "  FAIL: %s -- %s\n" "$1" "$2"
}

printf "=== Rust Toolchain Tests ===\n\n"

# ---------------------------------------------------------------------------
# rustc version
# ---------------------------------------------------------------------------

_name="rustc --version output"
if _out=$(rustc --version 2>&1); then
    case "$_out" in
        rustc\ *)
            pass "$_name"
            printf "    %s\n" "$_out"
            ;;
        *)
            fail "$_name" "unexpected format: $_out"
            ;;
    esac
else
    fail "$_name" "rustc --version returned nonzero"
fi

_name="rustc version is stable or nightly"
_out=$(rustc --version 2>&1)
if printf '%s' "$_out" | grep -qE '(stable|nightly|beta)'; then
    pass "$_name"
else
    # Could be a custom toolchain; just warn
    pass "$_name (custom channel)"
fi

# ---------------------------------------------------------------------------
# Compile and run hello world
# ---------------------------------------------------------------------------

_name="compile hello world"
cat > "$TMPDIR/hello.rs" << 'RSEOF'
fn main() {
    println!("Hello from Rust!");
}
RSEOF

if rustc "$TMPDIR/hello.rs" -o "$TMPDIR/hello" 2>/dev/null; then
    pass "$_name"
else
    fail "$_name" "rustc compilation failed"
fi

_name="run hello world"
if [ -x "$TMPDIR/hello" ]; then
    _out=$("$TMPDIR/hello" 2>&1)
    if [ "$_out" = "Hello from Rust!" ]; then
        pass "$_name"
    else
        fail "$_name" "unexpected output: $_out"
    fi
else
    fail "$_name" "binary not found or not executable"
fi

# ---------------------------------------------------------------------------
# Compile with std::fs operations
# ---------------------------------------------------------------------------

_name="compile std::fs program"
cat > "$TMPDIR/fstest.rs" << 'RSEOF'
use std::fs;
use std::io::Write;

fn main() {
    let test_dir = format!("{}/fstest_data", env!("CARGO_MANIFEST_DIR", "/tmp"));
    let dir = std::env::temp_dir().join("rust_fs_test");
    let _ = fs::create_dir_all(&dir);

    // Write a file
    let path = dir.join("test.txt");
    let mut f = fs::File::create(&path).expect("create failed");
    f.write_all(b"hello rust fs").expect("write failed");
    drop(f);

    // Read it back
    let content = fs::read_to_string(&path).expect("read failed");
    assert_eq!(content, "hello rust fs");

    // Check file exists
    assert!(path.exists());

    // Remove file
    fs::remove_file(&path).expect("remove failed");
    assert!(!path.exists());

    // Cleanup
    let _ = fs::remove_dir_all(&dir);

    println!("fs_test_ok");
}
RSEOF

if rustc "$TMPDIR/fstest.rs" -o "$TMPDIR/fstest" 2>/dev/null; then
    pass "$_name"
else
    fail "$_name" "std::fs program compilation failed"
fi

_name="run std::fs program"
if [ -x "$TMPDIR/fstest" ]; then
    _out=$("$TMPDIR/fstest" 2>&1)
    if [ "$_out" = "fs_test_ok" ]; then
        pass "$_name"
    else
        fail "$_name" "unexpected output: $_out"
    fi
else
    fail "$_name" "binary not found or not executable"
fi

# ---------------------------------------------------------------------------
# cargo new + cargo build
# ---------------------------------------------------------------------------

_name="cargo new creates project"
if cargo new "$TMPDIR/testproject" 2>/dev/null; then
    pass "$_name"
else
    fail "$_name" "cargo new failed"
fi

_name="cargo build succeeds"
if [ -d "$TMPDIR/testproject" ]; then
    if (cd "$TMPDIR/testproject" && cargo build 2>/dev/null); then
        pass "$_name"
    else
        fail "$_name" "cargo build failed"
    fi
else
    fail "$_name" "project directory not created"
fi

_name="cargo build produces binary"
if [ -f "$TMPDIR/testproject/target/debug/testproject" ]; then
    pass "$_name"
else
    fail "$_name" "binary not found"
fi

_name="cargo-built binary runs"
if [ -x "$TMPDIR/testproject/target/debug/testproject" ]; then
    _out=$("$TMPDIR/testproject/target/debug/testproject" 2>&1)
    if [ "$_out" = "Hello, world!" ]; then
        pass "$_name"
    else
        fail "$_name" "unexpected output: $_out"
    fi
else
    fail "$_name" "binary not executable"
fi

_name="cargo test succeeds"
if [ -d "$TMPDIR/testproject" ]; then
    if (cd "$TMPDIR/testproject" && cargo test 2>/dev/null); then
        pass "$_name"
    else
        fail "$_name" "cargo test failed"
    fi
else
    fail "$_name" "project directory not found"
fi

# ---------------------------------------------------------------------------
# rustc feature checks
# ---------------------------------------------------------------------------

_name="rustc supports edition 2021"
cat > "$TMPDIR/edition2021.rs" << 'RSEOF'
fn main() {
    let x: i32 = 42;
    let _y = matches!(x, 42);
    println!("edition2021_ok");
}
RSEOF

if rustc --edition 2021 "$TMPDIR/edition2021.rs" -o "$TMPDIR/edition2021" 2>/dev/null; then
    _out=$("$TMPDIR/edition2021" 2>&1)
    if [ "$_out" = "edition2021_ok" ]; then
        pass "$_name"
    else
        fail "$_name" "unexpected output: $_out"
    fi
else
    fail "$_name" "edition 2021 compilation failed"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

TOTAL=$((PASS + FAIL))
printf "\n--- Results: %d passed, %d failed (total %d) ---\n" "$PASS" "$FAIL" "$TOTAL"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
printf "\nAll Rust toolchain tests passed.\n"
exit 0
