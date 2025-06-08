#!/bin/bash
# Test runner script for RISC-V 64 architecture

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
ARCH="riscv64"
TARGET="riscv64gc-veridian"
TARGET_JSON="targets/riscv64gc-veridian.json"
TEST_TIMEOUT=30
QEMU_ARGS="-M virt -nographic -bios default"

echo -e "${BLUE}=== VeridianOS RISC-V 64 Test Runner ===${NC}"
echo ""

# Function to run a single test
run_test() {
    local test_name=$1
    local test_binary=$2
    
    echo -e "${YELLOW}Running test: $test_name${NC}"
    
    # Run the test with timeout and capture output
    if timeout $TEST_TIMEOUT qemu-system-riscv64 \
        -kernel $test_binary \
        $QEMU_ARGS \
        2>&1 | tee test_output.log | grep -q "All tests passed"; then
        
        echo -e "${GREEN}✓ $test_name passed${NC}"
        return 0
    else
        echo -e "${RED}✗ $test_name failed${NC}"
        echo "Test output:"
        cat test_output.log
        return 1
    fi
}

# Build tests
echo -e "${BLUE}Building tests...${NC}"
cargo test --no-run --target $TARGET_JSON \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem \
    2>&1 | tee build_output.log

# Find test binaries
TEST_BINARIES=$(find target/$TARGET/debug/deps -name "*-*" -type f ! -name "*.d" ! -name "*.rlib" | grep -v ".so" || true)

if [ -z "$TEST_BINARIES" ]; then
    echo -e "${YELLOW}No test binaries found. Trying to build basic_boot test...${NC}"
    
    # Build specific integration test
    cargo test --test basic_boot --no-run --target $TARGET_JSON \
        -Zbuild-std=core,compiler_builtins,alloc \
        -Zbuild-std-features=compiler-builtins-mem
    
    TEST_BINARY=$(find target/$TARGET/debug/deps -name "basic_boot-*" -type f ! -name "*.d" | head -1)
    
    if [ -n "$TEST_BINARY" ]; then
        run_test "basic_boot" "$TEST_BINARY"
    else
        echo -e "${RED}No test binaries found!${NC}"
        exit 1
    fi
else
    # Run all found tests
    FAILED=0
    for test_binary in $TEST_BINARIES; do
        test_name=$(basename $test_binary | cut -d'-' -f1)
        
        # Skip non-executable files
        if [ ! -x "$test_binary" ]; then
            continue
        fi
        
        if ! run_test "$test_name" "$test_binary"; then
            FAILED=$((FAILED + 1))
        fi
    done
    
    if [ $FAILED -gt 0 ]; then
        echo -e "${RED}$FAILED tests failed${NC}"
        exit 1
    fi
fi

# Clean up
rm -f test_output.log build_output.log

echo -e "${GREEN}All tests passed!${NC}"