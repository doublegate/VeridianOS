# Design Rationale

This document explains the **major design trade-offs** in VeridianOS and why they were chosen.

VeridianOS is a systems research artifact. Its design favors **clarity, correctness, and durability** over convenience, feature breadth, or short-term performance.

---

## 1. Capability-Based Design

### Choice
VeridianOS uses explicit capabilities to represent authority.

### Rationale
Capabilities make authority:
- explicit rather than ambient
- transferable only by design
- auditable in code and documentation

This avoids confused-deputy problems and allows reasoning about *what can happen*, not just *what is intended*.

### Trade-Off
Capability systems impose additional structure and complexity compared to global access models.

This complexity is accepted because it makes authority flow inspectable and enforceable.

---

## 2. Minimal Trusted Computing Base (TCB)

### Choice
Only components that must enforce invariants reside in the kernel.

### Rationale
A smaller TCB:
- reduces the surface area for failure
- simplifies reasoning about correctness
- makes verification and audit feasible

### Trade-Off
More functionality is pushed into services and userland, increasing IPC and design effort.

This is accepted to preserve isolation and long-term maintainability.

---

## 3. Correctness Over Performance

### Choice
Correctness and invariant preservation take precedence over performance optimization.

### Rationale
Performance can be measured and optimized later.  
Correctness failures invalidate all other system properties.

By prioritizing correctness early:
- performance work has a stable foundation
- optimizations do not undermine safety guarantees

### Trade-Off
Early implementations may appear slower or less feature-complete.

This is acceptable for a research system focused on durability.

---

## 4. Explicit Non-Goals

### Choice
VeridianOS explicitly avoids POSIX compatibility and general-purpose OS goals.

### Rationale
Compatibility layers obscure authority, blur isolation boundaries, and impose legacy semantics.

Explicit non-goals prevent accidental scope expansion and protect architectural clarity.

### Trade-Off
The system is not immediately usable for conventional workloads.

This is intentional.

---

## 5. Rust as the Implementation Language

### Choice
Rust is used for the majority of the system.

### Rationale
Rust enables:
- strong memory safety guarantees
- explicit handling of unsafety
- enforcement of ownership and lifetime at compile time

Rust’s type system complements capability-based design.

### Trade-Off
Rust does not eliminate all unsafety, especially in low-level systems code.

This is mitigated through strict unsafe policy and invariant binding.

---

## 6. Controlled Use of Unsafe Code

### Choice
Unsafe Rust is allowed only to enforce higher-level invariants.

### Rationale
Unsafe code is unavoidable in OS development, but unbounded unsafe use erodes trust.

By binding unsafe code to documented invariants:
- its necessity is clear
- its correctness is reviewable
- its scope remains limited

### Trade-Off
Development velocity may be slower due to stricter review and documentation requirements.

This is accepted to preserve system integrity.

---

## 7. Documentation as a First-Class Artifact

### Choice
Documentation is treated as normative and binding.

### Rationale
High-assurance systems fail when intent is implicit.

By making documentation authoritative:
- reviewers can reason about correctness without reading all code
- future contributors understand design constraints
- the system remains intelligible over time

### Trade-Off
Documentation requires ongoing maintenance and discipline.

This is considered part of the engineering cost, not overhead.

---

## 8. Teaching and Inspectability

### Choice
The system is designed to be inspectable and teachable.

### Rationale
Systems that cannot be explained are difficult to trust.

Designing for inspectability:
- surfaces hidden assumptions
- exposes failure modes
- improves long-term understanding

### Trade-Off
Some abstractions are more verbose or explicit than strictly necessary.

This verbosity is intentional.

---

## Summary

VeridianOS makes deliberate trade-offs in favor of:

- explicit authority
- enforceable invariants
- minimal trust
- long-term clarity

These choices reduce short-term convenience but increase confidence, auditability, and durability.

The system is designed to reward deep understanding rather than hide complexity.
