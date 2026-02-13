# Capability Flow into Services and Drivers

This document explains how **capabilities are created, transferred, and consumed**
by services and drivers in VeridianOS.

Diagram (normative): [`diagrams/architecture-capability-flow.mmd`](diagrams/architecture-capability-flow.mmd)  
Related: kernel mediation boundaries are summarized in [`diagrams/kernel-entry-points.mmd`](diagrams/kernel-entry-points.mmd).

This document is normative.

---

## 1. Capability Creation

Capabilities originate exclusively in the kernel.
Only the kernel may create, revoke, or transfer authority.

---

## 2. Capability Transfer

Capabilities may be transferred only via explicit, kernel-mediated IPC.
There is no ambient authority.

---

## 3. Services as Capability Routers

Services receive and mediate authority but do not originate it.
Failure must not leak authority.

---

## 4. Drivers and Hardware Capabilities

Drivers receive constrained hardware access capabilities.
Hardware is treated as an untrusted collaborator.

---

## 5. Capability Lifetime and Revocation

Capabilities have explicit lifetimes and must support revocation.
Dangling authority is an architectural failure.

---

## Summary

Capability flow defines how authority moves through VeridianOS and replaces implicit trust
with explicit, enforceable delegation.
