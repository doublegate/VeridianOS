# Kernel Entry Points

This document identifies and explains the **primary kernel entry points** in VeridianOS.

Diagram (normative): [`diagrams/kernel-entry-points.mmd`](diagrams/kernel-entry-points.mmd)

Kernel entry points define where control, authority, and trust are first exercised.
They form the **root of the trusted execution path** and must be understood before evaluating any higher-level behavior.

This document is normative.

---

## Purpose

Kernel entry points exist to:

- establish the initial trusted execution context
- enforce architectural invariants from first instruction
- mediate all transitions between privilege domains
- create the initial capability set

Any ambiguity at these boundaries is an architectural failure.

---

## Definition: Kernel Entry Point

A kernel entry point is any location where:

- execution enters the kernel from firmware, bootloader, or userland
- privilege level changes
- authority is first asserted or mediated

---

## 1. Boot-Time Entry

The boot-time entry establishes kernel control, trusted memory layout, and the initial execution environment.
It performs no policy decisions.

---

## 2. Interrupt and Exception Entry

Interrupt and exception entry points preserve isolation under asynchronous control flow
and must never bypass invariant enforcement.

---

## 3. System Call Entry

System call entry mediates all transitions from userland to kernel authority.
All inputs and capabilities must be validated explicitly.

---

## 4. Relationship to Invariants

Kernel entry points must uphold:
- I-1 (Authority Is Explicit)
- I-5 (Isolation Boundaries Are Architectural)
- I-14 (Minimal TCB)

---

## Summary

Kernel entry points define the security and correctness perimeter of VeridianOS.
