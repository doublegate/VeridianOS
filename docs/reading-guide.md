# Reading Guide for Reviewers

This guide is intended for **systems researchers, OS developers, and security reviewers** evaluating VeridianOS.

VeridianOS is not a tutorial kernel. It is a **research artifact** whose primary outputs are architectural clarity, invariants, and executable enforcement of design decisions.

---

## 1. What to Read First (Orientation)

Before reading any code:

1. **README.md**  
   Understand the project’s purpose, non-goals, and threat model.

2. **docs/invariants.md**  
   This is the most important document in the repository.  
   All correctness claims reduce to whether these invariants hold.

3. **docs/architecture.md**  
   Explains authority flow, isolation boundaries, and system structure.

If these documents are unclear, the implementation should be considered suspect.

---

## 2. How to Evaluate the Code

When reviewing code, ask the following questions:

- Does this component possess explicit authority for what it does?
- Which invariant(s) does this code uphold?
- Is the trust boundary clear?
- Could this failure mode violate isolation or ownership guarantees?

Code that works but violates an invariant is **incorrect by definition**.

---

## 3. Unsafe Code Review Path

Unsafe Rust is treated as an architectural mechanism, not an optimization.

When reviewing unsafe code:

1. Read **docs/unsafe-policy.md**
2. Locate the unsafe block
3. Verify that:
   - the referenced invariant exists
   - the justification is correct
   - assumptions are explicit and bounded

Unsafe code without invariant binding should be treated as a defect.

---

## 4. Suggested Code Entry Points

For architectural understanding:

- Kernel entry and initialization paths
- Capability creation and mediation logic
- IPC boundary definitions
- Memory ownership and mapping logic

Do not start with drivers or userland unless reviewing a specific subsystem.

---

## 5. What Is Normative vs Historical

Normative (authoritative):
- README.md
- docs/invariants.md
- docs/architecture.md
- docs/unsafe-policy.md

Historical or developmental:
- docs/status/PROJECT-STATUS.md
- docs/status/PHASE2-STATUS-SUMMARY.md
- docs/status/BOOTLOADER-UPGRADE-STATUS.md

If historical documents conflict with normative documents, the historical documents are wrong.

---

## 6. How to Provide Useful Feedback

The most valuable feedback focuses on:

- invariant completeness or correctness
- unclear authority flow
- unnecessary expansion of the trusted computing base
- places where assumptions are implicit rather than explicit

Feature requests and performance suggestions are secondary.

---

## Summary

VeridianOS should be evaluated as a **systems research artifact**, not a product.

If the invariants are sound, explicit, and enforced, the system is succeeding—even if features are incomplete.

Correctness is the primary metric.
