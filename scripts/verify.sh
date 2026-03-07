#!/bin/bash
# VeridianOS Formal Verification Runner
#
# Runs Kani proof harnesses and TLA+ model checking.
# Gracefully degrades if tools are not installed.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
VERIFY_DIR="$ROOT_DIR/verification"
TLA_DIR="$VERIFY_DIR/tla+"
KANI_DIR="$ROOT_DIR/kernel"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

KANI_PASS=0
KANI_FAIL=0
TLA_PASS=0
TLA_FAIL=0

echo "========================================"
echo " VeridianOS Formal Verification Suite"
echo "========================================"
echo ""

# --- Kani Proofs ---

echo "--- Kani Proof Harnesses ---"
echo ""

if command -v cargo-kani &>/dev/null || cargo kani --version &>/dev/null 2>&1; then
    echo "Kani detected. Running 38 proof harnesses..."
    echo ""

    cd "$KANI_DIR"

    HARNESSES=(
        proof_fast_path_register_integrity
        proof_send_receive_roundtrip
        proof_fifo_ordering
        proof_no_message_loss
        proof_channel_capacity_bound
        proof_channel_isolation
        proof_capability_required
        proof_zero_copy_no_overlap
        proof_deadlock_freedom
        proof_async_ring_buffer_safety
        proof_message_type_safety
        proof_notification_delivery
        proof_token_encoding_roundtrip
        proof_no_forgery
        proof_derivation_subset
        proof_cascading_revocation
        proof_generation_invalidation
        proof_rights_mask_operations
        proof_capability_isolation
        proof_revocation_completeness
        proof_pcr_extend_monotonic
        proof_pcr_extend_deterministic
        proof_measurement_log_ordered
        proof_boot_status_transitions
        proof_policy_decision_complete
        proof_hash_chain_integrity
        proof_pcr_no_reset
        proof_measurement_count_matches
        proof_no_double_allocation
        proof_dealloc_makes_available
        proof_buddy_split_correct
        proof_buddy_coalesce_correct
        proof_bitmap_buddy_threshold
        proof_frame_conservation
        proof_zone_dma_range
        proof_alignment_preserved
        proof_no_overlap
        proof_free_idempotent
    )

    for harness in "${HARNESSES[@]}"; do
        if cargo kani --harness "$harness" &>/dev/null; then
            echo -e "  ${GREEN}PASS${NC}  $harness"
            ((KANI_PASS++))
        else
            echo -e "  ${RED}FAIL${NC}  $harness"
            ((KANI_FAIL++))
        fi
    done

    cd "$ROOT_DIR"
else
    echo -e "${YELLOW}Kani not found.${NC} Install with:"
    echo "  cargo install --locked kani-verifier"
    echo "  cargo kani setup"
    echo ""
    echo "Skipping 38 Kani proof harnesses."
fi

echo ""

# --- TLA+ Specs ---

echo "--- TLA+ Model Checking ---"
echo ""

TLC_CMD=""
if command -v tlc &>/dev/null; then
    TLC_CMD="tlc"
elif [ -f "$ROOT_DIR/tla2tools.jar" ]; then
    TLC_CMD="java -jar $ROOT_DIR/tla2tools.jar"
elif [ -f "$VERIFY_DIR/tla2tools.jar" ]; then
    TLC_CMD="java -jar $VERIFY_DIR/tla2tools.jar"
elif [ -n "${TLA2TOOLS_JAR:-}" ] && [ -f "$TLA2TOOLS_JAR" ]; then
    TLC_CMD="java -jar $TLA2TOOLS_JAR"
fi

if [ -n "$TLC_CMD" ]; then
    echo "TLC detected. Running 6 specifications..."
    echo ""

    SPECS=(
        boot_chain
        capability_model
        ipc_protocol
        ipc_deadlock
        memory_allocator
        information_flow
    )

    cd "$TLA_DIR"

    for spec in "${SPECS[@]}"; do
        if [ ! -f "${spec}.tla" ] || [ ! -f "${spec}.cfg" ]; then
            echo -e "  ${YELLOW}SKIP${NC}  $spec (missing .tla or .cfg)"
            continue
        fi

        if $TLC_CMD -config "${spec}.cfg" "${spec}.tla" -deadlock &>/dev/null; then
            echo -e "  ${GREEN}PASS${NC}  $spec"
            ((TLA_PASS++))
        else
            echo -e "  ${RED}FAIL${NC}  $spec"
            ((TLA_FAIL++))
        fi
    done

    cd "$ROOT_DIR"
else
    echo -e "${YELLOW}TLC not found.${NC} Install with:"
    echo "  wget https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar"
    echo ""
    echo "Set TLA2TOOLS_JAR environment variable or place tla2tools.jar in the project root."
    echo ""
    echo "Skipping 6 TLA+ specifications."
fi

echo ""

# --- Summary ---

echo "========================================"
echo " Verification Summary"
echo "========================================"

TOTAL_PASS=$((KANI_PASS + TLA_PASS))
TOTAL_FAIL=$((KANI_FAIL + TLA_FAIL))
TOTAL=$((TOTAL_PASS + TOTAL_FAIL))

if [ "$TOTAL" -eq 0 ]; then
    echo -e "${YELLOW}No verification tools installed.${NC}"
    echo "Install Kani and/or TLC to run verifications."
else
    echo ""
    if [ "$KANI_PASS" -gt 0 ] || [ "$KANI_FAIL" -gt 0 ]; then
        echo "  Kani:  $KANI_PASS passed, $KANI_FAIL failed (of $((KANI_PASS + KANI_FAIL)))"
    fi
    if [ "$TLA_PASS" -gt 0 ] || [ "$TLA_FAIL" -gt 0 ]; then
        echo "  TLA+:  $TLA_PASS passed, $TLA_FAIL failed (of $((TLA_PASS + TLA_FAIL)))"
    fi
    echo "  Total: $TOTAL_PASS passed, $TOTAL_FAIL failed (of $TOTAL)"
    echo ""
    if [ "$TOTAL_FAIL" -eq 0 ]; then
        echo -e "${GREEN}All verifications passed.${NC}"
    else
        echo -e "${RED}Some verifications failed.${NC}"
        exit 1
    fi
fi
