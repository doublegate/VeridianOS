# Development Setup

This guide will help you set up your development environment for working on VeridianOS.

## Prerequisites

Before you begin, ensure your system meets these requirements:

- **Operating System**: Linux-based (Fedora, Ubuntu, Debian, or similar)
- **RAM**: 8GB minimum, 16GB recommended for faster builds
- **Disk Space**: 20GB+ free space
- **CPU**: Multi-core processor recommended for parallel builds
- **Internet**: Required for downloading dependencies

## Installing Rust

VeridianOS requires a specific Rust nightly toolchain. The project includes a `rust-toolchain.toml` file that automatically manages this for you.

```bash
# Install rustup if you haven't already
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Source the cargo environment
source $HOME/.cargo/env

# The correct toolchain will be installed automatically when you build
```

## System Dependencies

Install the required system packages for your distribution:

### Fedora/RHEL/CentOS

```bash
sudo dnf install -y \
    qemu qemu-system-x86 qemu-system-aarch64 qemu-system-riscv \
    gdb gdb-multiarch \
    gcc make binutils \
    grub2-tools xorriso mtools \
    git gh \
    mdbook
```

### Ubuntu/Debian

```bash
sudo apt-get update
sudo apt-get install -y \
    qemu-system-x86 qemu-system-arm qemu-system-misc \
    gdb gdb-multiarch \
    gcc make binutils \
    grub-pc-bin xorriso mtools \
    git gh \
    mdbook
```

### Arch Linux

```bash
sudo pacman -S \
    qemu qemu-arch-extra \
    gdb \
    gcc make binutils \
    grub xorriso mtools \
    git github-cli \
    mdbook
```

## Development Tools

Install the required Rust development tools:

```bash
# Clone the repository first
git clone https://github.com/doublegate/VeridianOS.git
cd VeridianOS

# Install all development tools automatically
just install-tools
```

This installs:
- `rust-src`: Rust standard library source (required for custom targets)
- `llvm-tools-preview`: LLVM tools for debugging symbols
- `bootimage`: Creates bootable disk images
- `cargo-xbuild`: Cross-compilation support
- `cargo-binutils`: Binary utilities
- `cargo-watch`: File watcher for development
- `cargo-audit`: Security vulnerability scanner

## Editor Setup

### VS Code

1. Install the [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extension
2. Install the [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb) extension for debugging

The project includes `.vscode/` configuration for optimal development experience.

### Vim/Neovim

For Vim/Neovim users, install:
- [rust.vim](https://github.com/rust-lang/rust.vim)
- [coc.nvim](https://github.com/neoclide/coc.nvim) with coc-rust-analyzer

### Emacs

For Emacs users:
- [rustic](https://github.com/brotzeit/rustic)
- [lsp-mode](https://github.com/emacs-lsp/lsp-mode) with rust-analyzer

## Verifying Your Setup

Run these commands to verify everything is installed correctly:

```bash
# Check Rust installation
rustc --version
cargo --version

# Check QEMU installation
qemu-system-x86_64 --version
qemu-system-aarch64 --version
qemu-system-riscv64 --version

# Check GDB installation
gdb --version
gdb-multiarch --version

# Build and run the kernel
just run
```

If the kernel boots successfully in QEMU, your development environment is ready!

## Troubleshooting

### Common Issues

1. **Rust toolchain errors**
   ```bash
   # Force reinstall the correct toolchain
   rustup toolchain install nightly-2025-01-15
   rustup override set nightly-2025-01-15
   ```

2. **Missing rust-src component**
   ```bash
   rustup component add rust-src llvm-tools-preview
   ```

3. **QEMU not found**
   - Ensure QEMU is in your PATH
   - Try using the full path: `/usr/bin/qemu-system-x86_64`

4. **Permission denied errors**
   - Ensure you have proper permissions in the project directory
   - Don't run cargo or just commands with sudo

### Getting Help

If you encounter issues:
1. Check the [Troubleshooting Guide](../project/troubleshooting.md)
2. Search existing [GitHub Issues](https://github.com/doublegate/VeridianOS/issues)
3. Join our [Discord server](https://discord.gg/veridian)
4. Open a new issue with detailed error messages

## Next Steps

Now that your environment is set up:
- Learn how to [build VeridianOS](./building.md)
- Try [running in QEMU](./running.md)
- Explore the [architecture](../architecture/overview.md)
- Start [contributing](../contributing/how-to.md)!