# VeridianOS Architecture

This document describes the **architectural structure** of VeridianOS.

Diagram (normative): [`diagrams/architecture-capability-flow.mmd`](diagrams/architecture-capability-flow.mmd)

It explains *what exists*, *why it exists*, and *how authority flows* through the system.  
Implementation details are included only where they enforce architectural invariants.

---

## 1. Architectural Overview

VeridianOS is a **capability-oriented operating system** with strong isolation boundaries and explicit authority flow.

The system is structured as a small trusted core surrounded by increasingly constrained components.

High-level goals:
- explicit authority
- minimal trusted computing base
- clear isolation
- inspectable behavior

---

## 2. Layered Structure

### 2.1 Kernel (Trusted Computing Base)

The kernel is responsible for:
- enforcing isolation boundaries
- managing memory ownership
- creating and mediating capabilities
- scheduling execution

The kernel is the **only component** allowed to:
- map physical memory
- manage address spaces
- create fundamental capabilities

The kernel must remain minimal.

---

### 2.2 Drivers

Drivers interact with hardware but do **not** implicitly trust that hardware.

Driver responsibilities:
- hardware access behind explicit privilege boundaries
- mediation through kernel-provided interfaces
- no direct authority escalation

Drivers do not bypass kernel enforcement.

---

### 2.3 Services

Services provide higher-level system functionality.

Properties:
- receive authority exclusively via capabilities
- do not execute with kernel privilege
- may fail independently

Services act as **capability routers**, not authorities.

---

### 2.4 Userland

Userland processes are intentionally constrained.

Userland properties:
- no ambient authority
- explicit resource access only
- isolation by default

Userland is where most experimentation should occur.

---

## 3. Capability Model

Capabilities in VeridianOS:
- represent authority, not identity
- are explicit objects
- are transferable only through kernel mediation

Capabilities may represent:
- memory regions
- communication endpoints
- hardware access
- service interfaces

Capabilities define *what can be done*, not *who can do it*.

---

## 4. Inter-Process Communication (IPC)

IPC is:
- capability-mediated
- explicit
- structured

Properties:
- endpoints are capabilities
- message passing does not imply shared memory
- shared memory requires explicit setup

IPC exists to preserve isolation while enabling cooperation.

---

## 5. Memory Architecture

### 5.1 Ownership

All memory has an owner.

Ownership rules:
- only the kernel assigns ownership
- ownership transfer is explicit
- revocation is supported

---

### 5.2 Address Spaces

Each component executes in its own address space unless explicitly designed otherwise.

Address space sharing is exceptional and deliberate.

---

### 5.3 Shared Memory

Shared memory:
- is capability-governed
- has explicit lifetime rules
- exists only when required

---

## 6. Boot and Trust Assumptions

The boot process establishes:
- initial kernel authority
- initial capability set
- trusted execution baseline

Trust assumptions are explicit and bounded.

Early boot code is part of the TCB and treated accordingly.

---

## 7. Failure Semantics

Failure is:
- expected
- localized
- observable

The system is designed so that:
- component failure does not imply system compromise
- recovery strategies can be reasoned about

---

## 8. Determinism and Debuggability

The architecture favors:
- deterministic execution paths
- explicit sources of non-determinism
- inspectable system state

This supports:
- debugging
- testing
- replay and analysis

---

## 9. Architectural Evolution

VeridianOS evolves by:
- extending capabilities
- refining enforcement mechanisms
- preserving invariants

Architecture changes must:
- preserve invariant correctness
- maintain clarity of authority flow
- avoid expanding the TCB unnecessarily

---

## 10. Relationship to Implementation

This document is **normative**.

If implementation details diverge from this architecture, the implementation is incorrect.

Implementation exists to serve architecture, not the reverse.

---

## Summary

VeridianOS is designed as a system where:
- authority is explicit
- isolation is enforced
- failure is contained
- behavior is inspectable

The architecture is intentionally constrained to support long-term correctness and understanding.
