# VeridianOS Architectural Invariants

This document defines the **non-negotiable invariants** of VeridianOS.

Diagrams (interpretation aids, not replacements for invariants):
- [`diagrams/architecture-capability-flow.mmd`](diagrams/architecture-capability-flow.mmd)
- [`diagrams/kernel-entry-points.mmd`](diagrams/kernel-entry-points.mmd)

An invariant is a property that **must always hold** for the system to be considered correct.  
If an invariant is violated, the system is incorrect regardless of observed behavior.

These invariants are architectural commitments, not implementation suggestions.

---

## 1. Authority and Capability Invariants

### I-1: Authority Is Explicit
No component may perform an action unless it possesses an explicit capability granting that authority.

There are no implicit permissions, ambient authority, or global access paths.

---

### I-2: Capabilities Are Unforgeable
Capabilities cannot be fabricated, duplicated, or modified by untrusted components.

Only trusted kernel mechanisms may create, revoke, or transfer capabilities.

---

### I-3: Capability Scope Is Minimal
Capabilities grant only the minimum authority required for their purpose.

Broad or multi-purpose capabilities are architectural failures.

---

### I-4: No Confused Deputies
A component must never exercise authority on behalf of another component unless explicitly delegated via capability transfer.

---

## 2. Isolation Invariants

### I-5: Isolation Boundaries Are Architectural
Isolation between kernel, drivers, services, and userland is enforced by design, not convention.

Cross-boundary interaction occurs only through well-defined interfaces.

---

### I-6: Components Cannot Bypass Boundaries
No component may directly access the internal state or memory of another component without explicit authorization.

---

### I-7: Failure Is Contained
Failure in one component must not silently corrupt or compromise other components.

Failure propagation must be explicit, observable, and bounded.

---

## 3. Memory and Ownership Invariants

### I-8: Memory Ownership Is Explicit
Every memory region has a clear owner at all times.

Ownership transfer is explicit and mediated by the kernel.

---

### I-9: No Aliasing Across Trust Boundaries
Memory must not be aliased across isolation boundaries unless explicitly intended and mediated.

Shared memory is a first-class, capability-governed construct.

---

### I-10: Lifetime Is Enforced
Memory access must not outlive the ownership or lifetime guarantees under which it was granted.

Use-after-free and temporal safety violations are architectural failures.

---

## 4. Unsafe Code Invariants

### I-11: Unsafe Code Is Exceptional
Unsafe Rust is permitted only where required to enforce higher-level invariants.

Unsafe code is never used for convenience or performance alone.

---

### I-12: Unsafe Code Is Localized
Unsafe operations must be tightly scoped, minimal, and isolated.

Large unsafe regions are considered design failures.

---

### I-13: Unsafe Code Must Uphold an Invariant
Every unsafe block must document:
- which invariant(s) it upholds
- why safe Rust is insufficient
- what assumptions it relies upon

---

## 5. Kernel and Trusted Computing Base (TCB)

### I-14: The TCB Is Minimal
Only code that must be trusted to enforce invariants belongs in the kernel.

All other functionality must live outside the TCB.

---

### I-15: Kernel Authority Is Deliberate
The kernel may only exercise authority required to:
- enforce isolation
- manage memory
- mediate capabilities
- schedule execution

---

## 6. Determinism and Inspectability

### I-16: System Behavior Is Inspectable
System state transitions must be observable and debuggable.

Hidden state transitions are architectural liabilities.

---

### I-17: Non-Determinism Is Explicit
Sources of non-determinism must be clearly identified and isolated.

Implicit or accidental non-determinism is a design failure.

---

## 7. Documentation and Verification

### I-18: Documentation Is Part of the System
Documentation that defines invariants, architecture, or assumptions is normative.

Code that contradicts documentation is incorrect.

---

### I-19: Invariants Are Verifiable
Invariants must be expressible in a form that supports:
- assertions
- tests
- static reasoning
- future formal verification

---

## 8. Evolution

### I-20: Invariants Are Stable
Invariants change rarely and deliberately.

Any proposed change to an invariant requires:
- explicit justification
- documentation updates
- review of affected subsystems

---

## Summary

VeridianOS is correct **only if all invariants hold**.

Performance, features, and convenience are secondary to invariant preservation.

These invariants define the identity of the system.
