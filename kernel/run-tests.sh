#!/bin/bash
# Script to run VeridianOS kernel tests
# This works around the duplicate lang items issue by building tests one at a time

set -e

echo "VeridianOS Test Runner"
echo "====================="
echo ""

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Track test results
PASSED=0
FAILED=0

# Function to run a single test binary
run_test() {
    local test_name=$1
    local test_path="tests/${test_name}.rs"
    
    if [ ! -f "$test_path" ]; then
        echo "Test file not found: $test_path"
        return 1
    fi
    
    echo -n "Running $test_name... "
    
    # Build the specific test
    if cargo test --test $test_name --target x86_64-unknown-none --no-run 2>/dev/null; then
        # Find the test binary
        local test_bin=$(find target/x86_64-unknown-none/debug/deps -name "${test_name}-*" -type f ! -name "*.d" | head -1)
        
        if [ -n "$test_bin" ]; then
            # Run the test in QEMU
            timeout 10s qemu-system-x86_64 \
                -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
                -serial stdio \
                -display none \
                -kernel "$test_bin" 2>/dev/null | grep -v "SeaBIOS" || true
                
            if [ ${PIPESTATUS[0]} -eq 0 ]; then
                echo -e "${GREEN}PASSED${NC}"
                ((PASSED++))
            else
                echo -e "${RED}FAILED${NC}"
                ((FAILED++))
            fi
        else
            echo -e "${RED}FAILED${NC} (binary not found)"
            ((FAILED++))
        fi
    else
        echo -e "${RED}FAILED${NC} (compilation error)"
        ((FAILED++))
    fi
}

# List of tests to run
TESTS=(
    "basic_boot"
    "memory_tests"
    "ipc_basic"
)

# Run each test
for test in "${TESTS[@]}"; do
    run_test "$test"
done

echo ""
echo "Test Summary"
echo "============"
echo -e "Passed: ${GREEN}${PASSED}${NC}"
echo -e "Failed: ${RED}${FAILED}${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
fi