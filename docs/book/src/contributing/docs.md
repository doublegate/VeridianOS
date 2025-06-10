# Documentation

This guide covers contributing to VeridianOS documentation, including writing standards, review processes, and maintenance procedures. Good documentation is essential for a successful open-source project, and we welcome contributions from developers, technical writers, and users.

## Documentation Architecture

### Documentation Structure

VeridianOS uses a multi-layered documentation approach:

```
docs/
├── book/                    # mdBook user documentation
│   ├── src/                # Markdown source files
│   └── book.toml           # mdBook configuration
├── api/                    # API reference documentation
├── design/                 # Design documents and specifications
├── tutorials/              # Step-by-step guides
├── rfcs/                   # Request for Comments (design proposals)
└── internal/               # Internal development documentation
```

### Documentation Types

**1. User Documentation (mdBook)**
- Getting started guides
- Architecture explanations  
- API usage examples
- Troubleshooting guides

**2. API Documentation (Rustdoc)**
- Automatically generated from code comments
- Function signatures and usage
- Examples and safety notes

**3. Design Documents**
- System architecture specifications
- Implementation plans
- Decision records

**4. Tutorials and Guides**
- Hands-on learning materials
- Best practices
- Common workflows

## Writing Standards

### Markdown Style Guide

Follow these conventions for consistent documentation:

#### Headers

```markdown
# Main Title (H1) - Only one per document
## Section (H2) - Main sections
### Subsection (H3) - Detailed topics
#### Sub-subsection (H4) - Specific details
```

#### Code Blocks

Always specify the language for syntax highlighting:

```rust
// Rust code example
fn example_function() -> Result<(), Error> {
    // Implementation
    Ok(())
}
```

```bash
# Shell commands
cargo build --target x86_64-unknown-veridian
```

```c
// C code for compatibility examples
int main() {
    printf("Hello, VeridianOS!\n");
    return 0;
}
```

#### Links and References

Use descriptive link text:

```markdown
<!-- Good -->
See the [memory management design](../design/MEMORY-ALLOCATOR-DESIGN.md) for details.

<!-- Avoid -->
See [here](../design/MEMORY-ALLOCATOR-DESIGN.md) for details.
```

#### Tables

Use tables for structured information:

```markdown
| Feature | Status | Target |
|---------|--------|--------|
| **Memory Management** | ✅ Complete | Phase 1 |
| **Process Management** | 🔄 In Progress | Phase 1 |
| **IPC System** | ✅ Complete | Phase 1 |
```

### Technical Writing Best Practices

#### Clarity and Concision

- Use clear, direct language
- Avoid jargon when possible
- Define technical terms on first use
- Keep sentences concise

#### Structure and Organization

- Use hierarchical organization
- Include table of contents for long documents
- Group related information together
- Provide clear section breaks

#### Code Examples

Always include complete, runnable examples:

```rust
// Complete example showing context
use veridian_std::capability::Capability;
use veridian_std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get filesystem capability
    let fs_cap = Capability::get("vfs")?;
    
    // Open file with capability
    let file = File::open_with_capability(fs_cap, "/etc/config")?;
    
    // Read contents
    let contents = file.read_to_string()?;
    println!("Config: {}", contents);
    
    Ok(())
}
```

#### Error Handling in Examples

Show proper error handling:

```rust
// Good: Shows error handling
match veridian_operation() {
    Ok(result) => {
        // Handle success
        println!("Operation succeeded: {:?}", result);
    }
    Err(e) => {
        // Handle error appropriately
        eprintln!("Operation failed: {}", e);
        return Err(e.into());
    }
}

// Avoid: Unwrapping without explanation
let result = veridian_operation().unwrap(); // Don't do this in docs
```

## API Documentation

### Rustdoc Standards

Follow these conventions for inline documentation:

#### Module Documentation

```rust
//! This module provides capability-based file system operations.
//!
//! VeridianOS uses capabilities to control access to file system resources,
//! providing fine-grained security while maintaining POSIX compatibility.
//!
//! # Examples
//!
//! ```rust
//! use veridian_fs::{Capability, File};
//!
//! let fs_cap = Capability::get("vfs")?;
//! let file = File::open_with_capability(fs_cap, "/etc/config")?;
//! ```
//!
//! # Security Considerations
//!
//! All file operations require appropriate capabilities. See the
//! [capability system documentation](../capability/index.html) for details.

use crate::capability::Capability;
```

#### Function Documentation

```rust
/// Opens a file using the specified capability.
///
/// This function provides capability-based file access, ensuring that
/// only processes with appropriate capabilities can access files.
///
/// # Arguments
///
/// * `capability` - The filesystem capability token
/// * `path` - The path to the file to open
/// * `flags` - File access flags (read, write, etc.)
///
/// # Returns
///
/// Returns a `File` handle on success, or an error if the operation fails.
///
/// # Errors
///
/// This function will return an error if:
/// - The capability is invalid or insufficient
/// - The file does not exist (when not creating)
/// - Permission is denied by the capability system
///
/// # Examples
///
/// ```rust
/// use veridian_fs::{Capability, File, OpenFlags};
///
/// let fs_cap = Capability::get("vfs")?;
/// let file = File::open_with_capability(
///     fs_cap,
///     "/etc/config",
///     OpenFlags::READ_ONLY
/// )?;
/// ```
///
/// # Safety
///
/// This function is safe to call from any context. All safety guarantees
/// are provided by the capability system.
pub fn open_with_capability(
    capability: Capability,
    path: &str,
    flags: OpenFlags,
) -> Result<File, FileError> {
    // Implementation
}
```

#### Type Documentation

```rust
/// A capability token that grants access to specific system resources.
///
/// Capabilities in VeridianOS are unforgeable tokens that represent
/// the authority to perform specific operations on system resources.
/// They provide fine-grained access control and are the foundation
/// of VeridianOS's security model.
///
/// # Design
///
/// Capabilities are 64-bit tokens with the following structure:
/// - Bits 0-31: Object ID (identifies the resource)
/// - Bits 32-47: Generation counter (for revocation)
/// - Bits 48-63: Rights bits (specific permissions)
///
/// # Examples
///
/// ```rust
/// // Request a capability from the system
/// let fs_cap = Capability::get("vfs")?;
///
/// // Derive a restricted capability
/// let readonly_cap = fs_cap.derive(Rights::READ_ONLY)?;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    token: u64,
}
```

### Documentation Testing

Ensure all code examples in documentation are tested:

```rust
/// # Examples
///
/// ```rust
/// # use veridian_fs::*;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let cap = Capability::get("vfs")?;
/// let file = File::open_with_capability(cap, "/test", OpenFlags::READ_ONLY)?;
/// # Ok(())
/// # }
/// ```
```

Run documentation tests with:

```bash
cargo test --doc
```

## mdBook Documentation

### Book Structure

The main documentation book follows this structure:

```
src/
├── introduction.md         # Project overview
├── getting-started/        # Initial setup guides
├── architecture/           # System design
├── api/                   # API guides
├── development/           # Development guides
├── advanced/              # Advanced topics
└── contributing/          # Contribution guides
```

### Cross-References

Use relative links for internal references:

```markdown
<!-- Reference to another chapter -->
For implementation details, see [Memory Management](../architecture/memory.md).

<!-- Reference to a specific section -->
The [IPC design](../architecture/ipc.md#zero-copy-implementation) explains
the zero-copy mechanism.

<!-- Reference to API documentation -->
See the [`Capability`](../../api/capability/struct.Capability.html) API
for usage details.
```

### Building the Book

```bash
# Install mdBook
cargo install mdbook

# Build the documentation
cd docs/book
mdbook build

# Serve locally for development
mdbook serve --open
```

### Book Configuration

Configure `book.toml` for optimal presentation:

```toml
[book]
title = "VeridianOS Documentation"
authors = ["VeridianOS Team"]
description = "Comprehensive documentation for VeridianOS"
src = "src"
language = "en"

[output.html]
theme = "theme"
default-theme = "navy"
preferred-dark-theme = "navy"
git-repository-url = "https://github.com/doublegate/VeridianOS"
edit-url-template = "https://github.com/doublegate/VeridianOS/edit/main/docs/book/{path}"

[output.html.search]
enable = true
limit-results = 30
teaser-word-count = 30
use-boolean-and = true
boost-title = 2
boost-hierarchy = 1
boost-paragraph = 1
expand = true
heading-split-level = 3

[output.html.print]
enable = true
```

## Contribution Workflow

### Getting Started

1. **Fork the Repository**
   ```bash
   git clone https://github.com/your-username/VeridianOS.git
   cd VeridianOS
   ```

2. **Create Documentation Branch**
   ```bash
   git checkout -b docs/your-improvement
   ```

3. **Make Changes**
   - Edit documentation files
   - Add new content
   - Update existing content

4. **Test Locally**
   ```bash
   # Test mdBook
   cd docs/book && mdbook serve
   
   # Test API docs
   cargo doc --open
   
   # Test code examples
   cargo test --doc
   ```

### Review Process

#### Self-Review Checklist

Before submitting, verify:

- [ ] **Accuracy**: All technical information is correct
- [ ] **Completeness**: No important information is missing
- [ ] **Clarity**: Content is understandable by target audience
- [ ] **Examples**: Code examples work and are tested
- [ ] **Links**: All internal and external links work
- [ ] **Grammar**: Proper spelling and grammar
- [ ] **Formatting**: Consistent markdown formatting
- [ ] **Images**: All images have alt text and are properly sized

#### Submission

```bash
# Commit changes
git add docs/
git commit -m "docs: improve capability system documentation

- Add comprehensive examples for capability derivation
- Clarify security implications
- Update API reference links"

# Push and create pull request
git push origin docs/your-improvement
```

#### Pull Request Template

Use this template for documentation PRs:

```markdown
## Documentation Changes

### Summary
Brief description of what documentation was changed and why.

### Changes Made
- [ ] New documentation added
- [ ] Existing documentation updated
- [ ] Dead links fixed
- [ ] Examples added/updated
- [ ] API documentation improved

### Target Audience
Who is the primary audience for these changes?
- [ ] New users
- [ ] Experienced developers
- [ ] API consumers
- [ ] Contributors

### Testing
- [ ] All code examples tested
- [ ] Links verified
- [ ] mdBook builds successfully
- [ ] Spell check completed

### Related Issues
Closes #XXX (if applicable)
```

## Maintenance and Updates

### Regular Maintenance Tasks

#### Monthly Reviews

- **Link Checking**: Verify all external links still work
- **Content Freshness**: Update version numbers and dates
- **Example Validation**: Ensure all examples still compile
- **Screenshot Updates**: Update UI screenshots if changed

#### Quarterly Audits

- **Completeness Review**: Identify missing documentation
- **User Feedback**: Review GitHub issues for documentation requests
- **Metrics Analysis**: Check documentation usage statistics
- **Reorganization**: Improve structure based on usage patterns

### Version Management

#### Release Documentation

For each release, update:

```bash
# Update version references
find docs/ -name "*.md" -exec sed -i 's/v0\.1\.0/v0.2.0/g' {} +

# Update changelog
echo "## Version 0.2.0" >> docs/CHANGELOG.md

# Tag documentation
git tag -a docs-v0.2.0 -m "Documentation for VeridianOS v0.2.0"
```

#### Deprecation Notices

Mark deprecated APIs clearly:

```rust
/// # Deprecated
///
/// This function is deprecated since version 0.2.0. Use
/// [`new_function`](fn.new_function.html) instead.
///
/// This function will be removed in version 1.0.0.
#[deprecated(since = "0.2.0", note = "use `new_function` instead")]
pub fn old_function() {
    // Implementation
}
```

### Internationalization

#### Translation Framework

Prepare for future translations:

```markdown
<!-- Use translation-friendly constructs -->
The system provides [security](security.md) through capabilities.

<!-- Avoid embedded screenshots with text -->
<!-- Use diagrams that can be easily translated -->
```

#### Content Organization

Structure content for translation:

- Keep sentences simple and direct
- Avoid idioms and cultural references
- Use consistent terminology
- Provide glossaries for technical terms

## Tools and Automation

### Documentation Tools

**mdBook**: Primary documentation platform
```bash
cargo install mdbook
cargo install mdbook-toc          # Table of contents
cargo install mdbook-linkcheck    # Link validation
```

**Rust Documentation**: API documentation
```bash
cargo doc --workspace --no-deps --open
```

**Link Checking**: Automated link validation
```bash
# Install link checker
cargo install lychee

# Check all documentation
lychee docs/**/*.md
```

### Automation Scripts

#### Document Generation Script

```bash
#!/bin/bash
# scripts/generate-docs.sh

set -e

echo "Generating VeridianOS documentation..."

# Build API documentation
echo "Building API documentation..."
cargo doc --workspace --no-deps

# Build user documentation
echo "Building user guide..."
cd docs/book
mdbook build

# Build design documents index
echo "Generating design document index..."
cd ../design
find . -name "*.md" | sort > index.txt

echo "Documentation generation complete!"
```

#### Documentation Testing

```bash
#!/bin/bash
# scripts/test-docs.sh

set -e

echo "Testing documentation..."

# Test code examples in docs
cargo test --doc

# Test mdBook builds
cd docs/book
mdbook test

# Check links
lychee --offline docs/**/*.md

# Spell check (if available)
if command -v aspell &> /dev/null; then
    find docs/ -name "*.md" -exec aspell check {} \;
fi

echo "Documentation tests passed!"
```

### Continuous Integration

Add documentation checks to CI:

```yaml
# .github/workflows/docs.yml
name: Documentation

on:
  push:
    paths: ['docs/**', '*.md']
  pull_request:
    paths: ['docs/**', '*.md']

jobs:
  docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install mdBook
        run: |
          curl -L https://github.com/rust-lang/mdBook/releases/latest/download/mdbook-x86_64-unknown-linux-gnu.tar.gz | tar xz
          echo "$PWD" >> $GITHUB_PATH
          
      - name: Build documentation
        run: |
          # Build user guide
          cd docs/book && mdbook build
          
          # Build API documentation
          cargo doc --workspace --no-deps
          
      - name: Check links
        run: |
          cargo install lychee
          lychee --offline docs/**/*.md
          
      - name: Deploy to GitHub Pages
        if: github.ref == 'refs/heads/main'
        uses: peaceiris/actions-gh-pages@v3
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./docs/book/book
```

## Style and Conventions

### Terminology

Use consistent terminology throughout documentation:

| Preferred | Avoid |
|-----------|-------|
| **VeridianOS** | Veridian OS, VeridianOS |
| **capability** | cap, permission |
| **microkernel** | micro kernel, μkernel |
| **user space** | userspace, user-space |
| **zero-copy** | zerocopy, zero copy |

### Voice and Tone

- **Active Voice**: "The system allocates memory" not "Memory is allocated"
- **Present Tense**: "The function returns..." not "The function will return..."
- **Second Person**: "You can configure..." not "One can configure..."
- **Confident**: "This approach provides..." not "This approach should provide..."

### Code Style

Use consistent code formatting in examples:

```rust
// Good: Consistent style
pub struct Example {
    field: u32,
}

impl Example {
    pub fn new() -> Self {
        Self { field: 0 }
    }
}

// Avoid: Inconsistent formatting
pub struct Example{
    field:u32,
}
impl Example{
    pub fn new()->Self{
        Self{field:0}
    }
}
```

## Getting Help

### Resources

- **Matrix Chat**: Join #veridian-docs:matrix.org for real-time help
- **GitHub Discussions**: Ask questions in the documentation category
- **Documentation Issues**: Report problems at https://github.com/doublegate/VeridianOS/issues

### Mentorship

New contributors can request documentation mentorship:

1. Comment on a "good first issue" in the documentation category
2. Mention your interest in learning technical writing
3. A maintainer will provide guidance and review

### Style Questions

When in doubt about style or conventions:

1. Check existing documentation for precedents
2. Ask in the documentation chat channel
3. Follow the principle of consistency over personal preference

Contributing to VeridianOS documentation helps make the project accessible to users and developers worldwide. Your contributions, whether fixing typos or writing comprehensive guides, are valuable and appreciated!