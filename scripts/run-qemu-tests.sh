#!/usr/bin/env bash
# run-qemu-tests.sh - QEMU-based test automation for VeridianOS
#
# Runs kernel test binaries in QEMU with serial output capture,
# timeout support, and pass/fail parsing. This bypasses the Rust
# toolchain limitation where -Zbuild-std + bare-metal targets
# cause duplicate lang items in cargo test.
#
# Each test binary is treated as a standalone kernel that prints
# [ok] or [failed] markers to serial output.
#
# Usage:
#   ./scripts/run-qemu-tests.sh [OPTIONS]
#
# Options:
#   -a, --arch ARCH     Target architecture: x86_64, aarch64, riscv64 (default: all)
#   -t, --timeout SECS  Timeout per test in seconds (default: 30)
#   -b, --build         Build test binaries before running (default: off)
#   -v, --verbose       Show full serial output for each test
#   -h, --help          Show this help message

set -euo pipefail

# Project root (one level up from scripts/)
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${PROJECT_ROOT}/target"

# Defaults
ARCH="all"
TIMEOUT=30
BUILD=false
VERBOSE=false

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' CYAN='' BOLD='' NC=''
fi

usage() {
    cat <<'USAGE'
Usage:
  ./scripts/run-qemu-tests.sh [OPTIONS]

Options:
  -a, --arch ARCH     Target architecture: x86_64, aarch64, riscv64 (default: all)
  -t, --timeout SECS  Timeout per test in seconds (default: 30)
  -b, --build         Build test binaries before running (default: off)
  -v, --verbose       Show full serial output for each test
  -h, --help          Show this help message
USAGE
}

log_info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
log_pass()  { echo -e "${GREEN}[PASS]${NC}  $*"; }
log_fail()  { echo -e "${RED}[FAIL]${NC}  $*"; }
log_skip()  { echo -e "${YELLOW}[SKIP]${NC}  $*"; }
log_bold()  { echo -e "${BOLD}$*${NC}"; }

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -a|--arch)    ARCH="$2"; shift 2 ;;
        -t|--timeout) TIMEOUT="$2"; shift 2 ;;
        -b|--build)   BUILD=true; shift ;;
        -v|--verbose) VERBOSE=true; shift ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# Validate architecture
case "$ARCH" in
    x86_64|aarch64|riscv64|all) ;;
    *) echo "Invalid architecture: $ARCH"; exit 1 ;;
esac

# Check QEMU availability
check_qemu() {
    local arch="$1"
    local qemu_bin
    case "$arch" in
        x86_64)  qemu_bin="qemu-system-x86_64" ;;
        aarch64) qemu_bin="qemu-system-aarch64" ;;
        riscv64) qemu_bin="qemu-system-riscv64" ;;
    esac

    if ! command -v "$qemu_bin" &>/dev/null; then
        log_skip "$arch: $qemu_bin not found"
        return 1
    fi
    return 0
}

# Get the target triple for an architecture
target_triple() {
    case "$1" in
        x86_64)  echo "x86_64-unknown-none" ;;
        aarch64) echo "aarch64-unknown-none" ;;
        riscv64) echo "riscv64gc-unknown-none-elf" ;;
    esac
}

# Build test binaries for an architecture
build_tests() {
    local arch="$1"
    local triple
    triple="$(target_triple "$arch")"

    log_info "Building test binaries for $arch ($triple)..."

    local build_args=(
        --target "$triple"
        -p veridian-kernel
        --features "test-kernel"
    )

    # x86_64 uses custom target JSON
    if [ "$arch" = "x86_64" ]; then
        build_args=(
            --target "${PROJECT_ROOT}/targets/x86_64-veridian.json"
            -p veridian-kernel
            -Zbuild-std=core,compiler_builtins,alloc
            --features "test-kernel"
        )
    fi

    if cargo build "${build_args[@]}" 2>&1; then
        log_info "Build succeeded for $arch"
    else
        log_fail "Build failed for $arch"
        return 1
    fi
}

# Find test binaries for an architecture
find_test_binaries() {
    local arch="$1"
    local triple
    triple="$(target_triple "$arch")"
    local bin_dir="${TARGET_DIR}/${triple}/debug"

    # x86_64 custom target uses a different path
    if [ "$arch" = "x86_64" ]; then
        bin_dir="${TARGET_DIR}/x86_64-veridian/debug"
        # x86_64 requires disk images (UEFI/BIOS) -- raw ELFs from deps/
        # cannot be booted directly. Only look for disk images.
        for img in "${bin_dir}/veridian-uefi.img" "${bin_dir}/veridian-bios.img"; do
            if [ -f "$img" ]; then
                echo "$img"
            fi
        done
        return
    fi

    if [ ! -d "$bin_dir" ]; then
        return
    fi

    # Find ELF executables in the deps directory that look like tests
    local deps_dir="${bin_dir}/deps"
    if [ -d "$deps_dir" ]; then
        find "$deps_dir" -maxdepth 1 -type f -executable ! -name '*.d' ! -name '*.rmeta' ! -name '*.rlib' 2>/dev/null || true
    fi

    # Also check for named test binaries directly
    for test_name in basic_boot ipc_integration_tests ipc_benchmarks scheduler_tests process_tests should_panic test_example; do
        local test_bin="${bin_dir}/${test_name}"
        if [ -f "$test_bin" ] && [ -x "$test_bin" ]; then
            echo "$test_bin"
        fi
    done
}

# Run a single test binary in QEMU
run_test() {
    local arch="$1"
    local binary="$2"
    local test_name
    test_name="$(basename "$binary")"

    local serial_log
    serial_log="$(mktemp /tmp/veridian-test-XXXXXX.log)"

    local qemu_cmd=()
    local qemu_exit_args=()

    case "$arch" in
        x86_64)
            if [[ "$binary" == *uefi* ]]; then
                # UEFI disk image requires OVMF firmware
                # Find OVMF firmware (prefer combined .fd for -bios flag)
                local ovmf="/usr/share/edk2/x64/OVMF.4m.fd"
                if [ ! -f "$ovmf" ]; then
                    ovmf="/usr/share/edk2/x64/OVMF.fd"
                fi
                if [ ! -f "$ovmf" ]; then
                    ovmf="/usr/share/OVMF/OVMF.fd"
                fi
                qemu_cmd=(
                    qemu-system-x86_64
                    -bios "$ovmf"
                    -drive "format=raw,file=${binary}"
                    -serial stdio
                    -display none
                    -no-reboot
                )
            else
                # BIOS disk image
                qemu_cmd=(
                    qemu-system-x86_64
                    -drive "format=raw,file=${binary}"
                    -serial stdio
                    -display none
                    -no-reboot
                )
            fi
            # ISA debug exit for x86_64 (port 0xf4)
            qemu_exit_args=(-device isa-debug-exit,iobase=0xf4,iosize=0x04)
            ;;
        aarch64)
            qemu_cmd=(
                qemu-system-aarch64
                -M virt
                -cpu cortex-a57
                -kernel "$binary"
                -serial stdio
                -display none
                -no-reboot
            )
            # AArch64 uses semihosting for exit
            qemu_exit_args=(-semihosting)
            ;;
        riscv64)
            qemu_cmd=(
                qemu-system-riscv64
                -M virt
                -kernel "$binary"
                -serial stdio
                -display none
                -no-reboot
            )
            # RISC-V uses SBI shutdown
            qemu_exit_args=()
            ;;
    esac

    # Run QEMU with timeout, capturing serial output
    local exit_code=0
    timeout "${TIMEOUT}s" "${qemu_cmd[@]}" "${qemu_exit_args[@]}" > "$serial_log" 2>&1 || exit_code=$?

    # Parse results -- check serial output first, since kernels that enter
    # an idle loop (HLT/WFI) will always be killed by timeout.
    local result="unknown"

    if grep -q 'BOOTOK' "$serial_log"; then
        result="boot-ok"
    elif grep -q '\[ok\]' "$serial_log"; then
        result="pass"
    elif grep -q '\[failed\]' "$serial_log" || grep -q 'BOOTFAIL' "$serial_log"; then
        result="fail"
    elif [ "$exit_code" -eq 33 ]; then
        # QEMU isa-debug-exit with value 0x10 -> exit code (0x10 << 1) | 1 = 33
        result="pass"
    elif [ "$exit_code" -eq 35 ]; then
        # QEMU isa-debug-exit with value 0x11 -> exit code (0x11 << 1) | 1 = 35
        result="fail"
    elif [ "$exit_code" -eq 124 ]; then
        # timeout(1) returns 124 -- only report as timeout if no markers found
        result="timeout"
    fi

    # Report result
    case "$result" in
        pass)    log_pass "$test_name" ;;
        boot-ok) log_pass "$test_name (boot ok)" ;;
        fail)    log_fail "$test_name" ;;
        timeout) log_fail "$test_name (timed out after ${TIMEOUT}s)" ;;
        *)       log_fail "$test_name (unknown result, exit code: $exit_code)" ;;
    esac

    # Show serial output in verbose mode or on failure
    if $VERBOSE || [ "$result" = "fail" ] || [ "$result" = "timeout" ] || [ "$result" = "unknown" ]; then
        echo "  --- serial output ---"
        sed 's/^/  | /' "$serial_log"
        echo "  --- end output ---"
    fi

    rm -f "$serial_log"

    # Return 0 for pass/boot-ok, 1 for fail/timeout/unknown
    case "$result" in
        pass|boot-ok) return 0 ;;
        *)            return 1 ;;
    esac
}

# Run tests for a single architecture
run_arch_tests() {
    local arch="$1"
    local passed=0
    local failed=0
    local skipped=0

    log_bold "--- $arch ---"

    if ! check_qemu "$arch"; then
        return 0
    fi

    if $BUILD; then
        build_tests "$arch" || return 1
    fi

    # Find test binaries
    local binaries
    binaries="$(find_test_binaries "$arch")"

    if [ -z "$binaries" ]; then
        log_skip "$arch: no test binaries found (build with --build or cargo build --features test-kernel)"
        return 0
    fi

    while IFS= read -r binary; do
        if run_test "$arch" "$binary"; then
            ((passed++))
        else
            ((failed++))
        fi
    done <<< "$binaries"

    local total=$((passed + failed))
    echo ""
    log_info "$arch results: $passed/$total passed, $failed failed"

    # Return failure if any test failed
    [ "$failed" -eq 0 ]
}

# Also support running the main kernel binary as a smoke test
run_kernel_boot_test() {
    local arch="$1"

    log_bold "--- $arch kernel boot test ---"

    if ! check_qemu "$arch"; then
        return 0
    fi

    local triple
    triple="$(target_triple "$arch")"
    local kernel="${TARGET_DIR}/${triple}/debug/veridian-kernel"

    # x86_64 uses a disk image (UEFI preferred, BIOS fallback) rather than raw ELF
    if [ "$arch" = "x86_64" ]; then
        kernel="${TARGET_DIR}/x86_64-veridian/debug/veridian-uefi.img"
        if [ ! -f "$kernel" ]; then
            kernel="${TARGET_DIR}/x86_64-veridian/debug/veridian-bios.img"
        fi
        if [ ! -f "$kernel" ]; then
            # Fall back to raw ELF path for error message
            kernel="${TARGET_DIR}/x86_64-veridian/debug/veridian-kernel"
        fi
    fi

    if [ ! -f "$kernel" ]; then
        log_skip "$arch: kernel binary not found at $kernel"
        return 0
    fi

    if run_test "$arch" "$kernel"; then
        return 0
    else
        return 1
    fi
}

# Main
main() {
    log_bold "VeridianOS QEMU Test Runner"
    log_info "Timeout: ${TIMEOUT}s per test"
    echo ""

    local total_passed=0
    local total_failed=0
    local arches=()

    if [ "$ARCH" = "all" ]; then
        arches=(x86_64 aarch64 riscv64)
    else
        arches=("$ARCH")
    fi

    for arch in "${arches[@]}"; do
        if run_arch_tests "$arch"; then
            ((total_passed++))
        else
            ((total_failed++))
        fi
        echo ""
    done

    # Summary
    log_bold "=== Summary ==="
    log_info "Architectures tested: ${#arches[@]}"

    if [ "$total_failed" -eq 0 ]; then
        log_pass "All architecture test suites passed"
        exit 0
    else
        log_fail "$total_failed architecture test suite(s) had failures"
        exit 1
    fi
}

main
