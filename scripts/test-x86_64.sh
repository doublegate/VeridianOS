#!/bin/bash
# Test runner script for x86_64 architecture

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
ARCH="x86_64"
TARGET="x86_64-veridian"
TARGET_JSON="targets/x86_64-veridian.json"
TEST_TIMEOUT=30
QEMU_ARGS="-device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -display none"

echo -e "${BLUE}=== VeridianOS x86_64 Test Runner ===${NC}"
echo ""

# Function to run a single test
run_test() {
    local test_name=$1
    local test_binary=$2
    
    echo -e "${YELLOW}Running test: $test_name${NC}"
    
    # Run the test with timeout
    if timeout $TEST_TIMEOUT qemu-system-x86_64 \
        -drive format=raw,file=$test_binary \
        $QEMU_ARGS \
        > test_output.log 2>&1; then
        
        # QEMU exit code 33 means success (exit code 0x10 << 1 | 1)
        if [ $? -eq 33 ]; then
            echo -e "${GREEN}✓ $test_name passed${NC}"
            return 0
        else
            echo -e "${RED}✗ $test_name failed${NC}"
            cat test_output.log
            return 1
        fi
    else
        echo -e "${RED}✗ $test_name timed out${NC}"
        return 1
    fi
}

# Build tests
echo -e "${BLUE}Building tests...${NC}"
cargo test --no-run --target $TARGET_JSON \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem \
    2>&1 | tee build_output.log

# Extract test binaries from build output
TEST_BINARIES=$(grep -oP 'target/[^\s]+/deps/[^\s]+(?=\))' build_output.log || true)

if [ -z "$TEST_BINARIES" ]; then
    echo -e "${YELLOW}No test binaries found. Building integration tests...${NC}"
    
    # Build specific integration tests
    cargo test --test basic_boot --no-run --target $TARGET_JSON \
        -Zbuild-std=core,compiler_builtins,alloc \
        -Zbuild-std-features=compiler-builtins-mem
    
    # Find the test binary
    TEST_BINARY=$(find target/$TARGET/debug/deps -name "basic_boot-*" -type f ! -name "*.d" | head -1)
    
    if [ -n "$TEST_BINARY" ]; then
        # Create bootimage for the test
        bootimage test $TEST_BINARY
        BOOTIMAGE="${TEST_BINARY/deps/bootimage}"
        
        run_test "basic_boot" "$BOOTIMAGE"
    else
        echo -e "${RED}No test binaries found!${NC}"
        exit 1
    fi
else
    # Run all found tests
    FAILED=0
    for test_binary in $TEST_BINARIES; do
        # Create bootimage for each test
        bootimage test $test_binary
        BOOTIMAGE="${test_binary/deps/bootimage}"
        
        test_name=$(basename $test_binary | cut -d'-' -f1)
        
        if ! run_test "$test_name" "$BOOTIMAGE"; then
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