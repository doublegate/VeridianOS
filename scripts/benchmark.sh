#!/bin/bash
# Benchmark runner for VeridianOS

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Default values
ARCH="x86_64"
BENCH_NAME=""
OUTPUT_DIR="benchmark_results"

# Help function
show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Run performance benchmarks for VeridianOS"
    echo ""
    echo "Options:"
    echo "  -a, --arch ARCH     Target architecture (x86_64, aarch64, riscv64) [default: x86_64]"
    echo "  -b, --bench NAME    Run specific benchmark (ipc, context, memory) [default: all]"
    echo "  -o, --output DIR    Output directory for results [default: benchmark_results]"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                     # Run all benchmarks for x86_64"
    echo "  $0 -a aarch64 -b ipc   # Run IPC benchmark for AArch64"
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            ARCH="$2"
            shift 2
            ;;
        -b|--bench)
            BENCH_NAME="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo -e "${RED}Error: Unknown option $1${NC}"
            show_help
            exit 1
            ;;
    esac
done

# Validate architecture
case $ARCH in
    x86_64|aarch64|riscv64)
        ;;
    *)
        echo -e "${RED}Error: Invalid architecture '$ARCH'${NC}"
        exit 1
        ;;
esac

# Map architecture to target
case $ARCH in
    x86_64)
        TARGET="x86_64-veridian"
        TARGET_JSON="targets/x86_64-veridian.json"
        QEMU="qemu-system-x86_64"
        QEMU_ARGS="-device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -display none"
        ;;
    aarch64)
        TARGET="aarch64-veridian"
        TARGET_JSON="targets/aarch64-veridian.json"
        QEMU="qemu-system-aarch64"
        QEMU_ARGS="-M virt -cpu cortex-a72 -nographic"
        ;;
    riscv64)
        TARGET="riscv64gc-veridian"
        TARGET_JSON="targets/riscv64gc-veridian.json"
        QEMU="qemu-system-riscv64"
        QEMU_ARGS="-M virt -nographic -bios default"
        ;;
esac

# Create output directory
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

echo -e "${CYAN}=== VeridianOS Benchmark Runner ===${NC}"
echo -e "${BLUE}Architecture: $ARCH${NC}"
echo -e "${BLUE}Output: $OUTPUT_DIR${NC}"
echo ""

# Function to run a benchmark
run_benchmark() {
    local bench_name=$1
    local bench_full_name=$2
    
    echo -e "${YELLOW}Building $bench_full_name benchmark...${NC}"
    
    # Build the benchmark
    cargo bench --bench $bench_name --no-run --target $TARGET_JSON \
        -Zbuild-std=core,compiler_builtins,alloc \
        -Zbuild-std-features=compiler-builtins-mem \
        2>&1 | tee build_output.log
    
    # Find the benchmark binary
    BENCH_BINARY=$(find target/$TARGET/release/deps -name "${bench_name}-*" -type f ! -name "*.d" | head -1)
    
    if [ -z "$BENCH_BINARY" ]; then
        echo -e "${RED}Failed to find benchmark binary!${NC}"
        return 1
    fi
    
    echo -e "${YELLOW}Running $bench_full_name benchmark...${NC}"
    
    # Run the benchmark and capture output
    local output_file="$OUTPUT_DIR/${bench_name}_${ARCH}_${TIMESTAMP}.log"
    
    if [ "$ARCH" = "x86_64" ]; then
        # Need bootimage for x86_64
        bootimage bench $BENCH_BINARY
        BOOTIMAGE="${BENCH_BINARY/deps/bootimage}"
        timeout 30 $QEMU -drive format=raw,file=$BOOTIMAGE $QEMU_ARGS > "$output_file" 2>&1 || true
    else
        timeout 30 $QEMU -kernel $BENCH_BINARY $QEMU_ARGS > "$output_file" 2>&1 || true
    fi
    
    # Extract and display results
    if grep -q "Target Analysis:" "$output_file"; then
        echo -e "${GREEN}Results:${NC}"
        sed -n '/^Results:/,/^Target Analysis:/p' "$output_file" | grep -v "Target Analysis:"
        echo ""
        echo -e "${GREEN}Target Analysis:${NC}"
        sed -n '/^Target Analysis:/,$p' "$output_file" | tail -n +3
    else
        echo -e "${RED}Benchmark failed or timed out!${NC}"
        tail -20 "$output_file"
    fi
    
    echo ""
}

# Determine which benchmarks to run
if [ -z "$BENCH_NAME" ]; then
    # Run all benchmarks
    echo -e "${BLUE}Running all benchmarks...${NC}"
    echo ""
    
    run_benchmark "ipc_latency" "IPC Latency"
    run_benchmark "context_switch" "Context Switch"
    run_benchmark "memory_allocation" "Memory Allocation"
else
    # Run specific benchmark
    case $BENCH_NAME in
        ipc)
            run_benchmark "ipc_latency" "IPC Latency"
            ;;
        context)
            run_benchmark "context_switch" "Context Switch"
            ;;
        memory)
            run_benchmark "memory_allocation" "Memory Allocation"
            ;;
        *)
            echo -e "${RED}Error: Unknown benchmark '$BENCH_NAME'${NC}"
            echo "Valid benchmarks: ipc, context, memory"
            exit 1
            ;;
    esac
fi

# Generate summary report
SUMMARY_FILE="$OUTPUT_DIR/summary_${ARCH}_${TIMESTAMP}.md"
echo "# VeridianOS Benchmark Summary" > "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"
echo "**Date**: $(date)" >> "$SUMMARY_FILE"
echo "**Architecture**: $ARCH" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"
echo "## Performance Targets" >> "$SUMMARY_FILE"
echo "- IPC Latency: < 5μs" >> "$SUMMARY_FILE"
echo "- Context Switch: < 10μs" >> "$SUMMARY_FILE"
echo "- Memory Allocation: < 1μs" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"
echo "## Results" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"

# Append results from each benchmark
for log in "$OUTPUT_DIR"/*_${ARCH}_${TIMESTAMP}.log; do
    if [ -f "$log" ]; then
        bench_name=$(basename "$log" | cut -d'_' -f1)
        echo "### $bench_name" >> "$SUMMARY_FILE"
        echo '```' >> "$SUMMARY_FILE"
        grep -A20 "Target Analysis:" "$log" | tail -n +3 >> "$SUMMARY_FILE" || echo "No results" >> "$SUMMARY_FILE"
        echo '```' >> "$SUMMARY_FILE"
        echo "" >> "$SUMMARY_FILE"
    fi
done

echo -e "${GREEN}Benchmark complete!${NC}"
echo -e "${GREEN}Summary saved to: $SUMMARY_FILE${NC}"

# Clean up
rm -f build_output.log