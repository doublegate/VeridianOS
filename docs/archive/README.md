# VeridianOS Documentation Archive

This directory contains historical documentation that is no longer actively maintained but preserved for reference. The archive is organized into subdirectories based on content type.

## Archive Structure

### 📚 `/book/`
**Purpose**: Historical mdBook configuration and update files  
**Contents**: 
- mdBook location updates
- Configuration change logs
- Book structure modifications

**Examples**:
- `BOOK-LOCATION-UPDATE-*.md` - Records of mdBook configuration changes
- `MDBOOK-UPDATE-SUMMARY-*.md` - Summaries of documentation updates

### 📝 `/doc_updates/`
**Purpose**: Historical documentation update summaries from development sessions  
**Contents**:
- Session-specific documentation changes
- Batch update summaries
- Documentation migration records

**Examples**:
- `DOCUMENTATION-UPDATE-SUMMARY-*.md` - Comprehensive update logs
- `DOCUMENTATION-UPDATE-Original.md` - Original master update file

### 🎨 `/format/`
**Purpose**: Historical formatting and linting fix records  
**Contents**:
- Code formatting change logs
- Linting rule updates
- Style guide evolution

**Examples**:
- `FORMATTING-LINTING-FIXES-*.md` - Detailed formatting fix records

### 🏗️ `/phase_0/`
**Purpose**: Phase 0 (Foundation & Tooling) completion documentation  
**Contents**:
- Phase 0 completion reports
- Achievement summaries
- Milestone checklists

**Examples**:
- `PHASE0-COMPLETION-SUMMARY.md` - Phase 0 achievement overview
- `PHASE0-COMPLETION-CHECKLIST.md` - Detailed task completion tracking

### 🚀 `/phase_1/`
**Purpose**: Phase 1 (Microkernel Core) completion documentation  
**Contents**:
- Phase 1 completion reports
- Implementation summaries
- Performance achievements
- Final polish reports

**Examples**:
- `PHASE1-COMPLETION-SUMMARY.md` - Phase 1 overview
- `PHASE1-COMPLETION-REPORT.md` - Detailed completion report
- `PHASE1-FINAL-POLISH-REPORT.md` - Final fixes and improvements
- `PHASE1-COMPLETION-CHECKLIST.md` - Task tracking

### 💻 `/sessions/`
**Purpose**: Development session summaries and technical achievements  
**Contents**:
- Session-specific implementation details
- Technical problem solutions
- Architecture decisions

**Examples**:
- `SESSION-UPDATE-*.md` - Individual session achievements

## Usage Guidelines

### When to Archive

Documents should be moved to the archive when:
- They contain historical information no longer relevant to active development
- They document completed phases or milestones
- They record one-time events (formatting fixes, migrations, etc.)
- They would clutter the active documentation directory

### Archive Naming Convention

Files in the archive typically follow these patterns:
- `{TYPE}-{DATE}.md` (e.g., `SESSION-UPDATE-20250615.md`)
- `{PHASE}{NUMBER}-{TYPE}.md` (e.g., `PHASE1-COMPLETION-SUMMARY.md`)
- `{TOPIC}-{DESCRIPTOR}.md` (e.g., `FORMATTING-LINTING-FIXES-2025-01-07.md`)

### Accessing Archive Content

While these files are not actively maintained, they provide valuable historical context:
- **Research**: Understanding past decisions and implementations
- **Debugging**: Tracing when specific changes were made
- **Learning**: Seeing the evolution of the project
- **Auditing**: Reviewing completed work and achievements

## Important Notes

1. **Do Not Modify**: Archive files should not be edited once moved here
2. **Reference Only**: These files are for historical reference only
3. **Active Docs**: For current documentation, see the parent `docs/` directory
4. **Summary Files**: Essential information is preserved in summary files in `docs/`

## Summary Files in Active Documentation

The following summary files in `docs/` contain the essential information extracted from archived content:
- `DOCUMENTATION-UPDATE-SUMMARY.md` - Current documentation standards
- `SESSION-UPDATE-SUMMARY.md` - Key technical achievements
- `PHASE0-SUMMARY.md` - Phase 0 essential infrastructure
- `PHASE1-SUMMARY.md` - Phase 1 core components
- `FORMATTING-LINTING-SUMMARY.md` - Code quality standards

---

*Last Updated: June 15, 2025*