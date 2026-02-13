# Unsafe Code Policy

This document defines the **policy governing all use of `unsafe` Rust** in VeridianOS.

Unsafe code is treated as a **controlled architectural mechanism**, not a convenience.  
Any violation of this policy renders the system **architecturally incorrect**, regardless of observed behavior.

---

## 1. Principle

Unsafe Rust exists in VeridianOS **only** to enforce higher-level safety, isolation, and correctness invariants that cannot be expressed in safe Rust alone.

Unsafe code is never used:
- for convenience
- for stylistic preference
- for premature optimization
- to bypass architectural constraints

---

## 2. Scope of Unsafe Code

Unsafe code is permitted **only** in the following contexts:

- Hardware interaction (e.g., MMIO, CPU instructions)
- Low-level memory management required to enforce ownership and isolation
- Boot and early initialization code
- Mechanisms that implement or enforce architectural invariants

All other code **must be written in safe Rust**.

---

## 3. Localization and Minimization

### U-1: Unsafe Blocks Must Be Minimal

- Unsafe blocks must be as small as possible
- Broad `unsafe fn` or `unsafe impl` usage is strongly discouraged
- Large unsafe regions are considered design failures

---

### U-2: Unsafe Must Be Localized

Unsafe code must be isolated to clearly identified modules or files.

Unsafe behavior must not “leak” across module boundaries.

---

## 4. Documentation Requirements

### U-3: Unsafe Must Be Justified

Every unsafe block must include a comment that explicitly states:

1. **Which architectural invariant(s) it upholds**
2. **Why safe Rust is insufficient**
3. **What assumptions are being relied upon**

Example (illustrative):

```rust
// SAFETY:
// Upholds I-8 (Memory Ownership) by ensuring this mapping is created
// only by the kernel during early boot.
// Safe Rust cannot express this invariant due to raw pointer usage.
// Assumes identity-mapped physical memory during this phase.
unsafe {
    ...
}
```

Undocumented unsafe code is incorrect by definition.

---

## 5. Invariant Binding

### U-4: Unsafe Code Must Bind to Invariants

Unsafe code must explicitly reference the invariant(s) defined in `docs/invariants.md` that it supports.

If no invariant can be named, the unsafe code is not permitted.

---

## 6. Unsafe Code Review Expectations

Unsafe code is subject to **stricter review** than safe code.

Review must verify:
- correctness of assumptions
- completeness of documentation
- absence of alternative safe designs
- preservation of all referenced invariants

Unsafe code is assumed guilty until proven necessary.

---

## 7. Prohibited Patterns

The following patterns are explicitly disallowed:

- Unsafe code for performance alone
- Unsafe abstractions that hide unsafety behind “safe-looking” APIs
- Unsafe access justified by “this is internal” reasoning
- Unsafe code copied from external sources without re-verification

---

## 8. Evolution and Refactoring

Unsafe code is expected to **shrink over time**, not grow.

As Rust evolves or better abstractions become available:
- unsafe code must be reevaluated
- safe replacements should be preferred
- invariants must remain preserved

---

## 9. Tooling and Enforcement (Aspirational)

Future enforcement mechanisms may include:
- linting rules that flag undocumented unsafe
- tests asserting invariant preservation
- static analysis or formal verification of unsafe regions

The architecture is designed to support these extensions.

---

## Summary

Unsafe Rust in VeridianOS is:
- rare
- deliberate
- justified
- documented
- invariant-bound

Unsafe code exists to **protect** correctness, not to compromise it.

Any unsafe code that does not meet this policy is incorrect by definition.
