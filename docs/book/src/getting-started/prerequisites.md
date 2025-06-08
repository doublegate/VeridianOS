# Prerequisites

Before building VeridianOS, ensure you have the following tools installed:

## Required Software

### Rust Toolchain
VeridianOS requires the nightly Rust compiler:

```bash
# Install rustup if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install the specific nightly version
rustup toolchain install nightly-2025-01-15
rustup component add rust-src llvm-tools-preview
```

### Build Tools
```bash
# Install required cargo tools
cargo install bootimage
cargo install cargo-xbuild
cargo install cargo-binutils
```

### Emulation and Testing
For running and testing VeridianOS:

```bash
# Debian/Ubuntu
sudo apt-get install qemu-system-x86 qemu-system-arm qemu-system-misc

# Fedora
sudo dnf install qemu-system-x86 qemu-system-aarch64 qemu-system-riscv

# macOS
brew install qemu
```

### Debugging Tools
```bash
# Install GDB with multiarch support
# Debian/Ubuntu
sudo apt-get install gdb-multiarch

# Fedora
sudo dnf install gdb

# macOS
brew install gdb
```

## Optional Tools

### Documentation
```bash
# Install mdBook for documentation
cargo install mdbook

# Install additional linters
npm install -g markdownlint-cli
```

### Development Environment
- **VS Code** with rust-analyzer extension
- **IntelliJ IDEA** with Rust plugin
- **Vim/Neovim** with rust.vim

## System Requirements

### Hardware
- **CPU**: x86_64, AArch64, or RISC-V host
- **RAM**: Minimum 8GB, 16GB recommended
- **Storage**: 10GB free space for builds

### Operating System
- Linux (recommended)
- macOS (with limitations)
- Windows via WSL2

## Verification

Verify your installation:

```bash
# Check Rust version
rustc +nightly-2025-01-15 --version

# Check QEMU
qemu-system-x86_64 --version

# Check GDB
gdb --version
```

## Next Steps

Once prerequisites are installed, proceed to [Building VeridianOS](./building.md).