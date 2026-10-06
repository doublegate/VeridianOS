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
