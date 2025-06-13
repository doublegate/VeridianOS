#!/bin/bash
# Script to run tests using the custom test framework

set -e

echo "Running VeridianOS Custom Test Framework"
echo "========================================"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default architecture
ARCH=${1:-x86_64}
TEST_NAME=${2:-"all"}

echo -e "${YELLOW}Architecture: $ARCH${NC}"
echo -e "${YELLOW}Test: $TEST_NAME${NC}"

# Build test based on architecture
case $ARCH in
    x86_64)
        TARGET="x86_64-unknown-none"
        QEMU_CMD="qemu-system-x86_64 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -display none"
        ;;
    aarch64)
        TARGET="aarch64-unknown-none"
        QEMU_CMD="qemu-system-aarch64 -M virt -cpu cortex-a57 -serial stdio -display none -semihosting"
        ;;
    riscv64)
        TARGET="riscv64gc-unknown-none-elf"
        QEMU_CMD="qemu-system-riscv64 -M virt -serial stdio -display none -bios none"
        ;;
    *)
        echo -e "${RED}Unknown architecture: $ARCH${NC}"
        exit 1
        ;;
esac

# Function to run a single test
run_test() {
    local test_name=$1
    echo -e "\n${YELLOW}Running test: $test_name${NC}"
    
    # Build the test
    if cargo test --target $TARGET --test $test_name --no-run --features test-kernel 2>&1 | grep -q "error"; then
        echo -e "${RED}Failed to build test: $test_name${NC}"
        return 1
    fi
    
    # Find the test binary
    TEST_BINARY=$(find target/$TARGET/debug/deps -name "${test_name}-*" -type f -executable | head -n1)
    
    if [ -z "$TEST_BINARY" ]; then
        echo -e "${RED}Test binary not found for: $test_name${NC}"
        return 1
    fi
    
    # Run the test in QEMU
    if [ "$ARCH" = "x86_64" ]; then
        # For x86_64, we need to create a bootimage
        cargo bootimage --target $TARGET --bin $test_name 2>/dev/null || true
        BOOTIMAGE="target/$TARGET/debug/bootimage-${test_name}.bin"
        if [ -f "$BOOTIMAGE" ]; then
            TEST_BINARY=$BOOTIMAGE
            QEMU_CMD="qemu-system-x86_64 -drive format=raw,file=$TEST_BINARY -device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -display none"
        fi
    fi
    
    # Run test with timeout
    timeout 30s $QEMU_CMD -kernel $TEST_BINARY
    EXIT_CODE=$?
    
    # Check exit code
    case $EXIT_CODE in
        33) # (0x10 << 1) | 1 = 33 for success
            echo -e "${GREEN}Test passed: $test_name${NC}"
            return 0
            ;;
        35) # (0x11 << 1) | 1 = 35 for failure
            echo -e "${RED}Test failed: $test_name${NC}"
            return 1
            ;;
        124)
            echo -e "${RED}Test timeout: $test_name${NC}"
            return 1
            ;;
        *)
            echo -e "${YELLOW}Test exited with code: $EXIT_CODE${NC}"
            return 1
            ;;
    esac
}

# Run tests
if [ "$TEST_NAME" = "all" ]; then
    # Run all custom tests
    TESTS=("test_example" "basic_boot" "ipc_integration_tests" "scheduler_tests" "process_tests")
    PASSED=0
    FAILED=0
    
    for test in "${TESTS[@]}"; do
        if run_test $test; then
            ((PASSED++))
        else
            ((FAILED++))
        fi
    done
    
    echo -e "\n${YELLOW}========== Test Summary ==========${NC}"
    echo -e "${GREEN}Passed: $PASSED${NC}"
    echo -e "${RED}Failed: $FAILED${NC}"
    
    if [ $FAILED -eq 0 ]; then
        echo -e "${GREEN}All tests passed!${NC}"
        exit 0
    else
        echo -e "${RED}Some tests failed!${NC}"
        exit 1
    fi
else
    # Run single test
    if run_test $TEST_NAME; then
        exit 0
    else
        exit 1
    fi
fi