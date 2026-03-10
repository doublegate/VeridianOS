# VeridianOS Documentation Archive

This directory contains historical documentation that is no longer actively maintained but preserved for reference.

*Last Updated: March 10, 2026*

## Archive Structure

### phase_specs/
Original phase specification documents from June 2025 (14K-55K lines each). These planning docs have been superseded by the mdBook chapters in `docs/book/src/phases/` and the completed implementation.

- `00-PHASE-0-FOUNDATION.md` through `06-PHASE-6-ADVANCED-FEATURES.md`

### sessions/
Development session logs and summaries from 2025-2026 development.

### reports/
One-off completion reports, summaries, and status snapshots:
- Phase completion summaries (PHASE0-SUMMARY, PHASE1-SUMMARY, etc.)
- Integration and implementation reports
- Technical change logs, project status snapshots
- Branch comparison, remediation reports

### ai-analysis/
AI-generated analysis and speculation documents (UPPER-KEBAB naming):
- `CLAUDE-4-VERIDIAN-FUTURE-DEV.md`, `CLAUDE4-IMPL-OUTLINE.md`
- `GPT-4O-VERIDIAN-FUTURE-DEV.md`, `GPT-4O-VERIDIAN-DEV-PLAN.pdf`
- `GROK-3-VERIDIAN-FUTURE-DEV.md`

### book/
Historical mdBook configuration and update files.

### doc_updates/
Historical documentation update summaries from development sessions.

### format/
Historical formatting and linting fix records.

### phase_0/ and phase_1/
Phase completion documentation for the earliest phases.

### Other
- `GEMINI.md` - Historical project context document (v0.5.7 era)

## Current Documentation

Active documentation is maintained in:
- **Repository root**: README.md, CHANGELOG.md, CONTRIBUTING.md, SECURITY.md, CLAUDE.md
- **docs/**: Reference guides, troubleshooting, porting guides
- **docs/book/**: mdBook (published to GitHub Pages)
- **docs/design/**: Authoritative subsystem design specifications (Memory, IPC, Scheduler, Capability)
- **to-dos/**: Active TODO tracking (completed phase TODOs archived in `to-dos/archive/`)

## Usage Guidelines

- **Do Not Modify**: Archive files should not be edited once moved here
- **Reference Only**: These files are for historical reference only
- **Active Docs**: For current documentation, see the parent `docs/` directory and `docs/book/`
