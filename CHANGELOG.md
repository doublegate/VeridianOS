# Changelog

All notable changes to VeridianOS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial project structure and build system
- Comprehensive documentation for all development phases
- Architecture overview and design principles
- API reference documentation
- Development and contribution guidelines
- Testing strategy and framework design
- Troubleshooting guide
- Project logos and branding assets

### Documentation
- Phase 0: Foundation and tooling setup guide
- Phase 1: Microkernel core implementation guide
- Phase 2: User space foundation guide
- Phase 3: Security hardening guide
- Phase 4: Package ecosystem guide
- Phase 5: Performance optimization guide
- Phase 6: Advanced features and GUI guide

### Project Setup
- Cargo workspace configuration
- Custom target specifications for x86_64, AArch64, and RISC-V
- Just command runner integration
- CI/CD pipeline configuration
- Development environment setup scripts

## [0.0.1] - TBD

### Planned for Initial Release
- Basic x86_64 boot capability
- Minimal kernel initialization
- Serial console output
- Basic memory detection
- Simple round-robin scheduler
- Initial testing framework

### Known Issues
- No driver support yet
- No user space support
- Limited to single CPU
- No file system
- No networking

## Versioning Scheme

VeridianOS follows Semantic Versioning:

- **MAJOR** version (X.0.0): Incompatible API changes
- **MINOR** version (0.X.0): Backwards-compatible functionality additions  
- **PATCH** version (0.0.X): Backwards-compatible bug fixes

### Pre-1.0 Versioning

While in pre-1.0 development:
- Minor version bumps may include breaking changes
- Patch versions are for bug fixes only
- API stability not guaranteed until 1.0.0

### Version Milestones

- **0.1.0** - Basic microkernel functionality
- **0.2.0** - Process and memory management
- **0.3.0** - IPC and capability system
- **0.4.0** - User space support
- **0.5.0** - Driver framework
- **0.6.0** - File system support
- **0.7.0** - Network stack
- **0.8.0** - Security features
- **0.9.0** - Package management
- **1.0.0** - First stable release

[Unreleased]: https://github.com/veridian-os/veridian/compare/main...HEAD