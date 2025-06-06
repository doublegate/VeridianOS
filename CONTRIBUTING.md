# Contributing to VeridianOS

First off, thank you for considering contributing to VeridianOS! It's people like you that will make VeridianOS a great operating system.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [How Can I Contribute?](#how-can-i-contribute)
- [Development Process](#development-process)
- [Coding Standards](#coding-standards)
- [Commit Guidelines](#commit-guidelines)
- [Pull Request Process](#pull-request-process)
- [Community](#community)

## Code of Conduct

This project and everyone participating in it is governed by our [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to [conduct@veridian-os.org](mailto:conduct@veridian-os.org).

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/VeridianOS.git
   cd VeridianOS
   ```
3. **Add upstream remote**:
   ```bash
   git remote add upstream https://github.com/doublegate/VeridianOS.git
   ```
4. **Set up development environment** following [DEVELOPMENT-GUIDE.md](docs/DEVELOPMENT-GUIDE.md)

## How Can I Contribute?

### Reporting Bugs

Before creating bug reports, please check existing issues to avoid duplicates. When creating a bug report, include:

- **Clear title and description**
- **Steps to reproduce**
- **Expected behavior**
- **Actual behavior**
- **System information** (OS, architecture, Rust version)
- **Relevant logs or error messages**

Use the bug report template when available.

### Suggesting Enhancements

Enhancement suggestions are tracked as GitHub issues. When creating an enhancement suggestion, include:

- **Clear title and description**
- **Rationale** - Why would this be useful?
- **Detailed explanation** of the enhancement
- **Possible implementation** approach (if you have ideas)

### Code Contributions

#### First-Time Contributors

Look for issues labeled with:
- `good first issue` - Simple issues good for newcomers
- `help wanted` - Issues where we need community help
- `documentation` - Documentation improvements

#### Areas Needing Contributions

- **Drivers**: Network, storage, and device drivers
- **Architecture Ports**: RISC-V support improvements
- **Testing**: Unit tests, integration tests, fuzzing
- **Documentation**: Tutorials, guides, API docs
- **Performance**: Optimizations and benchmarks
- **Security**: Security audits and hardening

### Documentation

Documentation improvements are always welcome! This includes:

- Fixing typos and grammar
- Clarifying existing documentation
- Adding examples
- Writing tutorials
- Translating documentation

## Development Process

### 1. Branch Strategy

- `main` - Stable branch, all tests passing
- `develop` - Development branch, PRs target here
- `feature/*` - Feature branches
- `fix/*` - Bug fix branches
- `docs/*` - Documentation branches

### 2. Setting Up Development Branch

```bash
# Sync with upstream
git fetch upstream
git checkout develop
git merge upstream/develop

# Create feature branch
git checkout -b feature/my-feature
```

### 3. Making Changes

1. **Write code** following our coding standards
2. **Add tests** for new functionality
3. **Update documentation** as needed
4. **Run tests locally**:
   ```bash
   just test
   ```
5. **Check formatting**:
   ```bash
   just fmt-check
   ```
6. **Run lints**:
   ```bash
   just clippy
   ```

### 4. Testing Requirements

- All new code must have tests
- Maintain or improve code coverage
- Tests must pass on all supported architectures
- Include both positive and negative test cases

## Coding Standards

### Rust Style Guide

We follow the official [Rust Style Guide](https://github.com/rust-dev-tools/fmt-rfcs/blob/master/guide/guide.md) with these additions:

#### Naming Conventions

- **Types**: `PascalCase`
- **Functions/Methods**: `snake_case`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Modules**: `snake_case`
- **Feature flags**: `kebab-case`

#### Documentation

Every public item must have documentation:

```rust
/// Allocates a physical memory frame.
/// 
/// # Returns
/// 
/// Returns `Some(Frame)` if successful, `None` if out of memory.
/// 
/// # Example
/// 
/// ```
/// let frame = allocator.allocate_frame()?;
/// ```
pub fn allocate_frame(&mut self) -> Option<Frame> {
    // Implementation
}
```

#### Error Handling

- Use `Result<T, Error>` for fallible operations
- Create specific error types using `thiserror`
- Provide helpful error messages
- Never use `.unwrap()` in non-test code

#### Unsafe Code

- Minimize unsafe code
- Document all safety requirements
- Isolate unsafe code in dedicated modules
- Provide safe wrappers

Example:
```rust
/// Dereferences a raw pointer.
/// 
/// # Safety
/// 
/// - `ptr` must be valid for reads
/// - `ptr` must be properly aligned
/// - The memory must not be mutated during access
unsafe fn read_raw(ptr: *const u8) -> u8 {
    // Safety: Caller ensures preconditions
    *ptr
}
```

### Architecture-Specific Code

- Place in `arch/<architecture>/` directories
- Use conditional compilation
- Provide common traits/interfaces
- Document architecture requirements

### Performance Considerations

- Profile before optimizing
- Document performance-critical code
- Prefer safe code unless performance requires unsafe
- Add benchmarks for performance-critical paths

## Commit Guidelines

We follow [Conventional Commits](https://www.conventionalcommits.org/):

### Format

```
<type>(<scope>): <subject>

<body>

<footer>
```

### Types

- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `test`: Test additions or modifications
- `build`: Build system changes
- `ci`: CI configuration changes
- `chore`: Other changes (e.g., dependency updates)

### Examples

```
feat(mm): implement huge page support

Add support for 2MB and 1GB huge pages in the memory manager.
This improves TLB efficiency for large allocations.

Closes #123
```

```
fix(scheduler): resolve race condition in thread wake

A race condition could occur when waking a thread that was
simultaneously being migrated to another CPU. This patch adds
proper locking to prevent the race.

Fixes #456
```

### Commit Best Practices

- Keep commits atomic and focused
- Write clear, descriptive messages
- Reference issues when applicable
- Sign commits with GPG when possible

## Pull Request Process

### Before Submitting

1. **Update from upstream**:
   ```bash
   git fetch upstream
   git rebase upstream/develop
   ```

2. **Run all checks**:
   ```bash
   just ci-checks
   ```

3. **Update documentation** if needed

4. **Add tests** for new functionality

5. **Update CHANGELOG.md** with your changes

### Submitting a Pull Request

1. **Push to your fork**:
   ```bash
   git push origin feature/my-feature
   ```

2. **Create Pull Request** via GitHub UI

3. **Fill out PR template** completely

4. **Link related issues**

5. **Request reviews** from maintainers

### PR Requirements

- [ ] All CI checks pass
- [ ] Code follows style guidelines
- [ ] Tests added/updated
- [ ] Documentation updated
- [ ] CHANGELOG.md updated
- [ ] Commits are clean and well-described
- [ ] PR description explains changes

### Review Process

1. **Automated checks** run first
2. **Maintainer review** for code quality
3. **Address feedback** promptly
4. **Squash commits** if requested
5. **Merge** once approved

### After Merge

- Delete your feature branch
- Update your local repository
- Celebrate your contribution! 🎉

## Communication

### Getting Help

- **Discord**: [#dev-help](https://discord.gg/veridian)
- **Matrix**: #veridian-dev:matrix.org
- **Mailing List**: dev@veridian-os.org

### Discussions

- **Architecture**: Use GitHub Discussions
- **Features**: Create an RFC (Request for Comments)
- **Bugs**: Use issue tracker

### Weekly Meetings

- **Time**: Thursdays at 18:00 UTC
- **Platform**: Discord voice channel
- **Agenda**: Posted in #meeting-agenda

## Recognition

Contributors are recognized in several ways:

- Listed in [CONTRIBUTORS.md](CONTRIBUTORS.md)
- Mentioned in release notes
- Special roles in Discord
- Contributor badge on forum

## Development Tips

### Building Faster

```bash
# Use sccache
export RUSTC_WRAPPER=sccache

# Limit parallel jobs if low on RAM
cargo build -j 2

# Build only specific component
cargo build -p veridian-kernel
```

### Debugging

```bash
# Enable debug logging
RUST_LOG=debug just run

# Run with GDB
just gdb

# Use QEMU monitor
just run -- -monitor stdio
```

### Testing

```bash
# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run benchmarks
cargo bench
```

## Questions?

If you have questions not covered here:

1. Check the [FAQ](docs/FAQ.md)
2. Ask on [Discord](https://discord.gg/veridian)
3. Email [dev@veridian-os.org](mailto:dev@veridian-os.org)

Thank you for contributing to VeridianOS! Your efforts help build a better operating system for everyone.

---

**Remember**: The best way to get started is to pick a small issue and dive in. We're here to help!