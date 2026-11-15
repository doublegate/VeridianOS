# ADR 0028: licensing by component, and a CLA

- Status: proposed
- Date: 2026-11-15
- Effective: from the commit that merges this change, ahead of v0.28.0 (the maintainer moved it forward from v0.29.0 on 2026-11-15, so that the work not yet published, the system tier, modify_ldt and the sockets work, is published under these terms). v0.28.0 is the first release under them. v0.27.0 and every commit published before this change stay MIT OR Apache-2.0.
- The maintainer decided, on 2026-11-15:
  - to relicense by component, as recommended;
  - to keep commercial licensing open;
  - to make it effective at once, before v0.28.0 (first planned for v0.29.0).

  The texts below (the exception, the CLA, the path map) are this ADR's, and wait for the maintainer's acceptance.

## Context

VeridianOS has been MIT OR Apache-2.0 throughout. That lets anyone ship a closed, modified VeridianOS kernel, as FreeBSD code went into closed systems without changes coming back. The maintainer wants to safeguard the work, and to be able to sell commercial licenses, as Genode Labs does with Genode (AGPL-3.0 plus commercial licenses).

The comparable systems license as follows (`docs/PRIOR-ART.md`, `docs/OS-COMPARISON.md`):
- **Linux:** GPL-2.0-only, with the syscall note keeping programs out.
- **seL4:** GPL-2.0 kernel, BSD user space.
- **Genode:** AGPL-3.0 plus commercial.
- **Asterinas:** MPL-2.0.
- **Redox, Fuchsia:** permissive.

On `main` the human author of every commit is the maintainer, with AI co-authorship recorded in trailers, so no other person's consent is needed to relicense.

## Decision

1. **The kernel, the system servers and the Linux runtime are GPL-3.0-or-later with the VeridianOS user-space exception** (`LICENSES/LicenseRef-VeridianOS-syscall-exception.txt`, an additional permission under GPL-3.0 section 7).
   - This covers `kernel/`, `userland/init/`, and the server and runtime directories as they are created: `servers/`, `drivers/`, `userland/servers/`, `userland/linuxd/`, `userland/vinit/`.
   - **GPL-3.0 rather than GPL-2.0:**
     - Its patent licence.
     - Its "installation information" rule, which stops devices that lock out modified versions.
     - Its termination cure.
     - It accepts the Apache-2.0 code the project already has. GPL-2.0 does not.
   - **"Or later"** follows the FSF's recommendation, so a future GPL can apply. The maintainer, who holds the copyright and the CLA rights, can choose differently for later releases.
   - **The exception** states that a program using the kernel or servers only through system calls, IPC protocols, or the published user-space headers and libraries is not a work based on them. This covers unmodified Linux programs and separate driver servers. Code linked into or loaded into the kernel is not covered, nor are modifications.

2. **Everything that others build against, or that is not part of the trusted base, stays MIT OR Apache-2.0:**
   - `abi/` (the generated syscall numbers among it);
   - `userland/libc`, `libm`, `ld-veridian`, `libcurses`, `libwayland`, `rust-std`;
   - `crates/` (`veridian-cbpf`, `veridian-landlock`, `veridian-keys`, and from v0.29 `abi-linux`, `personality-linux`, `veridian-rt` and the IDL);
   - the user programs (`vsh`, `coreutils`, `programs`), the desktop integration code, the tools, the tests, the build system and the documentation.

   Permissive code may be used in the GPL parts. The reverse is not allowed.

3. **Third-party files keep their licenses:**
   - the vendored Linux syscall tables (`GPL-2.0-only WITH Linux-syscall-note`);
   - the patches to other projects (`LicenseRef-Upstream-Patch`: each under the license of the files it changes).

   The planned Linux Driver Container (ADR 0030) is a separate GPL-2.0 program.

4. **REUSE is the record.** `REUSE.toml` maps every path to its license, the texts are in `LICENSES/`, and `reuse lint` runs in CI and fails on any file without a license.
   - SPDX 2.3 cannot name a custom exception with `WITH`, and the REUSE tool does not yet accept SPDX 3.0's `AdditionRef-`. So the GPL parts are marked `GPL-3.0-or-later AND LicenseRef-VeridianOS-syscall-exception`; the exception's text says it is a section 7 additional permission.
   - The root `COPYING` is GPL-3.0; `LICENSE-MIT` and `LICENSE-APACHE` stay.
   - `Cargo.toml` gives the kernel `GPL-3.0-or-later`. Cargo's field cannot express the exception, so `REUSE.toml` is authoritative.

5. **A contributor licence agreement keeps commercial licensing open.**
   - Every contributor signs `CLA.md` once, through the CLA Assistant workflow, which records signatures in the repository.
   - It grants the maintainer a broad, irrevocable licence to the contribution, including the right to relicense it under any terms, commercial ones included. It also grants a patent licence.
   - Contributors keep their copyright, so it is a licence, not an assignment (as the Harmony agreements allow).
   - Changes to the Linux Driver Container are GPL-2.0 and are not covered by the relicensing grant, since they derive from Linux.

6. **Commercial licences** for the GPL parts are offered by the maintainer (`COMMERCIAL.md`), as Genode Labs offers them. They are agreed case by case. The public licences are unaffected.

## Consequences

**What this gives:**
- A closed, modified VeridianOS kernel can no longer be shipped under the public licence: changes to the GPL parts must be published with the binaries, and devices must allow modified versions to be installed.
- Applications, Linux programs, driver servers and other projects stay free to use VeridianOS's interfaces and crates under permissive terms, so the system does not frighten off application or hardware vendors.
- The maintainer can sell licences to vendors who cannot accept the GPL.

**What it costs:**
- Outside contributions need a signed CLA before they are merged. Some contributors decline CLAs; the DCO alternative would have closed off commercial licensing.
- GPL-3.0 code cannot be combined with GPL-2.0-only code in one work. LICENSING.md already keeps Linux code out of the kernel, so nothing changes in practice.
- Two licences in one tree need care when code moves between directories. Moving code from a GPL directory into a permissive one needs the maintainer, who holds the CLA rights.
- **Enforcement:** much of the code was written with AI assistance. Under current US Copyright Office guidance, material generated without enough human authorship may not be protected by copyright, which would weaken enforcement of any licence for those parts. The maintainer's design, review, selection and arrangement, and the code the maintainer wrote or shaped, are protected. The commit history and ADRs record that work.
- This is the project's reading of the licences, not legal advice. A licensing lawyer reviews the exception, the CLA and `COMMERCIAL.md` before v0.28.0 is tagged.

**Rejected alternatives:**
- **GPL-2.0-only:** cannot take the existing Apache-2.0 code, and has no patent licence or anti-lockdown terms.
- **AGPL-3.0:** its network clause adds little for a kernel, and would weigh on any server that embeds VeridianOS components.
- **MPL-2.0 for the kernel:** weaker protection, since modified files stay open but can be combined into closed products. It was the alternative if adoption mattered more than blocking closed forks.
- **One licence for everything:** copyleft on the ABI, libc and crates would bind every program and frighten off vendors.
- **DCO without a CLA:** simpler for contributors, but it would rule out commercial licensing, which the maintainer wants kept open.
