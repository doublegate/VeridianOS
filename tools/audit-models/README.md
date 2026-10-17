# audit-models

Host-side `std` programs that accompany `docs/PERFORMANCE_AND_QUALITY_AUDIT_MATRIX.md`.

**These are models, not measurements of VeridianOS.** The program imports nothing from
`veridian-kernel`; each benchmark re-implements the structure under discussion (for example a
single-mutex frame cache versus per-CPU caches) and times the re-implementation. Use the
figures as illustrations of an algorithmic choice. Kernel before/after numbers come from the
in-kernel `perf` shell command and are recorded in `docs/PERFORMANCE-REPORT.md`. See
`docs/audit/AUDIT-VERIFICATION-2026-10-05.md` for per-finding verification.

```bash
cd tools/audit-models
cargo run --release          # run all models
cargo test --release         # the same models as #[test]s
```

This crate was `tests/audit_benchmarks.rs` at the repository root, where no Cargo target
compiled it (the root manifest is a virtual workspace).

## Models proposed on 2026-10-10 (not adopted)

Another agent's unmerged session added 15 models (22 test functions) for the 2026-10-10 matrix
(`docs/audit/AUDIT-INTAKE-2026-10-10.md`). They were reviewed on 2026-10-15 and not adopted:

- 7 test functions repeat another under a second name.
- 9 assert nothing (proof-of-concept bodies that only run the flawed and fixed versions).
- The thresholds were lowered until the suite passed: the targeted TLB shootdown model passes at
  a 0.5x "speed-up", i.e. when the fixed path is twice as slow; the barrier model compares two
  timings that are always equal.
- The findings they illustrate are now fixed in the kernel with kernel tests, which is the
  evidence that counts: BlockFS miss I/O and write-back (`fs/blockfs`), IPC and capability
  authority (`cap::tests`, `ipc::sync`, `cap::revocation`), POSIX shm resize (`ipc::posix_shm`),
  COW mapping lookup (`mm::vas`). The rest are latent paths tracked in the intake (D4, D5, v0.29).

The proposal is archived with the rest of that session's work, outside the repository.
