# Changelog

The authoritative changelog for VeridianOS is maintained in the repository root:

**[CHANGELOG.md](https://github.com/doublegate/VeridianOS/blob/main/CHANGELOG.md)**

## Version-to-Phase Mapping

| Version Range | Phase | Description |
|---------------|-------|-------------|
| v0.1.0 | 0 | Foundation & Tooling |
| v0.2.0-v0.2.5 | 1 | Microkernel Core |
| v0.3.0-v0.3.5 | 2-3 | User Space + Security |
| v0.4.0-v0.4.9 | 4-4.5 | Packages + Shell |
| v0.5.0-v0.5.13 | T7+5+5.5 | Self-Hosting + Performance + Bridge |
| v0.6.0-v0.6.4 | 6 | Advanced Features & GUI |
| v0.7.0 | 6.5 | Rust Compiler + vsh Shell |
| v0.7.1-v0.10.0 | 7 | Production Readiness (6 Waves) |
| v0.10.1-v0.10.6 | -- | Integration audit + bug fixes |
| v0.11.0-v0.16.0 | 7.5 | Follow-On Features (8 Waves) |
| v0.16.2-v0.16.3 | 5+8 | Phase 5 completion + Next-Gen (8 Waves) |
| v0.16.4-v0.17.1 | -- | Tech debt remediation (3 tiers) |
| v0.18.0-v0.20.3 | -- | Final integration + GUI fixes |
| v0.21.0 | -- | Performance benchmarks + verification |
| v0.22.0 | 9 | KDE Plasma 6 Porting Infrastructure |
| v0.23.0 | 10 | KDE Limitations Remediation |
| v0.24.0 | 11 | KDE Default Desktop Integration |
| v0.25.0 | 12 | KDE Cross-Compilation |
| v0.25.1 | -- | KDE Session Launch Fix |

## Latest Release

**v0.25.1** (March 10, 2026) - KDE session launch fix: direct ELF binary execution fallback chain. kwin_wayland loads into Ring 3 (4 LOAD segments, ~66MB VA). Stripped rootfs 180MB.
