# How to Contribute

Thank you for your interest in contributing to VeridianOS! This guide will help you get started with contributing code, documentation, or ideas to the project.

## Code of Conduct

First and foremost, all contributors must adhere to our [Code of Conduct](https://github.com/doublegate/VeridianOS/blob/main/CODE_OF_CONDUCT.md). We are committed to providing a welcoming and inclusive environment for everyone.

## Ways to Contribute

### 1. Code Contributions

#### Finding Issues
- Look for issues labeled [`good first issue`](https://github.com/doublegate/VeridianOS/labels/good%20first%20issue)
- Check [`help wanted`](https://github.com/doublegate/VeridianOS/labels/help%20wanted) for more challenging tasks
- Review the [TODO files](https://github.com/doublegate/VeridianOS/tree/main/to-dos) for upcoming work

#### Before You Start
1. Check if someone is already working on the issue
2. Comment on the issue to claim it
3. Discuss your approach if it's a significant change
4. For major features, wait for design approval

#### Development Process
1. Fork the repository
2. Create a feature branch: `git checkout -b feature/your-feature-name`
3. Make your changes following our coding standards
4. Write or update tests
5. Update documentation if needed
6. Commit with descriptive messages
7. Push to your fork
8. Submit a pull request

### 2. Documentation Contributions

Documentation is crucial for VeridianOS! You can help by:
- Fixing typos or unclear explanations
- Adding examples and tutorials
- Improving API documentation
- Translating documentation (future)

### 3. Testing Contributions

Help improve our test coverage:
- Write unit tests for untested code
- Add integration tests
- Create benchmarks
- Report bugs with reproducible examples

### 4. Ideas and Feedback

Your ideas matter! Share them through:
- GitHub Issues for feature requests
- Discussions for general ideas
- Discord for real-time chat
- Mailing list for longer discussions

## Coding Standards

### Rust Style Guide

We follow the standard Rust style guide with some additions:

```rust
// Use descriptive variable names
let frame_allocator = FrameAllocator::new();  // Good
let fa = FrameAllocator::new();               // Bad

// Document public items
/// Allocates a contiguous range of physical frames.
/// 
/// # Arguments
/// * `count` - Number of frames to allocate
/// * `flags` - Allocation flags (e.g., ZONE_DMA)
/// 
/// # Returns
/// Physical address of first frame or error
pub fn allocate_frames(count: usize, flags: AllocFlags) -> Result<PhysAddr, AllocError> {
    // Implementation
}

// Use explicit error types
#[derive(Debug)]
pub enum AllocError {
    OutOfMemory,
    InvalidSize,
    InvalidAlignment,
}

// Prefer const generics over magic numbers
const PAGE_SIZE: usize = 4096;
const MAX_ORDER: usize = 11;
```

### Architecture-Specific Code

Keep architecture-specific code isolated:

```rust
// In arch/x86_64/mod.rs
pub fn init_gdt() {
    // x86_64-specific GDT initialization
}

// In arch/mod.rs
#[cfg(target_arch = "x86_64")]
pub use x86_64::init_gdt;
```

### Safety and Unsafe Code

- Minimize `unsafe` blocks
- Document safety invariants
- Prefer safe abstractions

```rust
// Document why unsafe is needed and why it's safe
/// Writes to the VGA buffer at 0xB8000.
/// 
/// # Safety
/// - VGA buffer must be mapped
/// - Must be called with interrupts disabled
unsafe fn write_vga(offset: usize, value: u16) {
    let vga_buffer = 0xB8000 as *mut u16;
    vga_buffer.add(offset).write_volatile(value);
}
```

## Testing Guidelines

### Test Organization

```rust
// Unit tests go in the same file
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_single_frame() {
        let mut allocator = FrameAllocator::new();
        let frame = allocator.allocate(1).unwrap();
        assert_eq!(frame.size(), PAGE_SIZE);
    }
}

// Integration tests go in tests/
// tests/memory_integration.rs
```

### Test Coverage

Aim for:
- 80%+ code coverage
- All public APIs tested
- Edge cases covered
- Error paths tested

## Pull Request Process

### Before Submitting

1. **Run all checks locally**:
   ```bash
   just fmt-check
   just clippy
   just test
   ```

2. **Update documentation**:
   - Add/update rustdoc comments
   - Update relevant .md files
   - Add examples if applicable

3. **Write a good commit message**:
   ```
   component: Brief description (50 chars max)

   Longer explanation of what changed and why. Wrap at 72 characters.
   Reference any related issues.

   Fixes #123
   ```

### PR Requirements

Your PR must:
- Pass all CI checks
- Have a clear description
- Reference related issues
- Include tests for new features
- Update documentation
- Follow coding standards

### Review Process

1. Automated CI runs checks
2. Maintainer reviews code
3. Address feedback
4. Maintainer approves
5. PR is merged

## Development Tips

### Building Specific Architectures

```bash
# Build for x86_64
just build-arch x86_64

# Build for AArch64
just build-arch aarch64

# Build for RISC-V
just build-arch riscv64
```

### Running Tests

```bash
# Run all tests
just test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Debugging

```bash
# Debug x86_64
just debug-x86_64

# Debug AArch64
just debug-aarch64

# Debug RISC-V
just debug-riscv64
```

## Getting Help

If you need help:

1. **Read the documentation**: Check if it's already explained
2. **Search issues**: Someone might have asked before
3. **Ask on Discord**: Quick questions and discussions
4. **Open an issue**: For bugs or unclear documentation
5. **Mailing list**: For design discussions

## Recognition

All contributors are recognized in our [CONTRIBUTORS.md](https://github.com/doublegate/VeridianOS/blob/main/docs/community/CONTRIBUTORS.md) file. We appreciate every contribution, no matter how small!

## License

By contributing, you agree that your contributions will be licensed under the same terms as VeridianOS (MIT/Apache 2.0 dual license).

Thank you for helping make VeridianOS better! 🦀
