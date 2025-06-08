#!/usr/bin/env bash
# Development environment setup script for VeridianOS

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}=== VeridianOS Development Setup ===${NC}"
echo ""

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# 1. Check prerequisites
echo "📋 Checking prerequisites..."

MISSING_DEPS=0

if ! command_exists git; then
    echo -e "${RED}  ❌ git not found${NC}"
    MISSING_DEPS=1
else
    echo -e "${GREEN}  ✓ git $(git --version | cut -d' ' -f3)${NC}"
fi

if ! command_exists rustc; then
    echo -e "${RED}  ❌ Rust not found${NC}"
    MISSING_DEPS=1
else
    echo -e "${GREEN}  ✓ Rust $(rustc --version | cut -d' ' -f2)${NC}"
fi

if ! command_exists cargo; then
    echo -e "${RED}  ❌ Cargo not found${NC}"
    MISSING_DEPS=1
else
    echo -e "${GREEN}  ✓ Cargo $(cargo --version | cut -d' ' -f2)${NC}"
fi

if [ $MISSING_DEPS -eq 1 ]; then
    echo ""
    echo -e "${RED}Missing dependencies. Please install them first.${NC}"
    exit 1
fi

# 2. Install git hooks
echo ""
echo "🔗 Installing git hooks..."
if [ -f "scripts/install-hooks.sh" ]; then
    bash scripts/install-hooks.sh
else
    echo -e "${YELLOW}  ⚠️  Git hooks installer not found${NC}"
fi

# 3. Configure git settings
echo ""
echo "⚙️  Configuring git settings..."

# Set up commit signing (optional)
if command_exists gpg; then
    if [ -z "$(git config --get user.signingkey)" ]; then
        echo -e "${YELLOW}  ℹ️  No GPG key configured for commit signing${NC}"
        echo "  To enable commit signing:"
        echo "    1. git config --global user.signingkey YOUR_KEY_ID"
        echo "    2. git config --global commit.gpgsign true"
    else
        echo -e "${GREEN}  ✓ GPG signing configured${NC}"
    fi
fi

# Configure git aliases for common tasks
echo ""
echo "🎯 Setting up useful git aliases..."

git config --local alias.co checkout
git config --local alias.br branch
git config --local alias.ci commit
git config --local alias.st status
git config --local alias.unstage 'reset HEAD --'
git config --local alias.last 'log -1 HEAD'
git config --local alias.visual '!gitk'
git config --local alias.amend 'commit --amend'
git config --local alias.graph 'log --oneline --graph --decorate'

echo -e "${GREEN}  ✓ Git aliases configured${NC}"

# 4. Verify Rust toolchain
echo ""
echo "🦀 Verifying Rust toolchain..."

REQUIRED_TOOLCHAIN="nightly-2025-01-15"
CURRENT_TOOLCHAIN=$(rustc --version | grep -oE 'nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}' || echo "stable")

if [ "$CURRENT_TOOLCHAIN" != "$REQUIRED_TOOLCHAIN" ]; then
    echo -e "${YELLOW}  ⚠️  Wrong toolchain active${NC}"
    echo "  Current: $CURRENT_TOOLCHAIN"
    echo "  Required: $REQUIRED_TOOLCHAIN"
    echo ""
    echo "  To install and use the correct toolchain:"
    echo "    rustup toolchain install $REQUIRED_TOOLCHAIN"
    echo "    rustup override set $REQUIRED_TOOLCHAIN"
else
    echo -e "${GREEN}  ✓ Correct toolchain active${NC}"
fi

# Check for required components
echo "  Checking components..."
COMPONENTS=("rust-src" "llvm-tools-preview" "rustfmt" "clippy")
for comp in "${COMPONENTS[@]}"; do
    if rustup component list | grep -q "$comp (installed)"; then
        echo -e "${GREEN}    ✓ $comp${NC}"
    else
        echo -e "${YELLOW}    ⚠️  $comp not installed${NC}"
        echo "       Run: rustup component add $comp"
    fi
done

# 5. Create local development config
echo ""
echo "📝 Creating local development configuration..."

# Create .env file if it doesn't exist
if [ ! -f ".env" ]; then
    cat > .env << 'EOF'
# Local development environment variables
# DO NOT COMMIT THIS FILE

# Enable verbose output
RUST_BACKTRACE=1
RUST_LOG=debug

# Git hook configuration
RUN_CLIPPY_PRECOMMIT=0  # Set to 1 to enable (slow)
RUN_TESTS_PREPUSH=1     # Set to 0 to disable

# Development settings
QEMU_DISPLAY=none       # Set to 'gtk' for GUI
QEMU_SERIAL=stdio       # Serial output to console
EOF
    echo -e "${GREEN}  ✓ Created .env file${NC}"
else
    echo -e "${GREEN}  ✓ .env file already exists${NC}"
fi

# 6. Summary
echo ""
echo -e "${GREEN}=== Setup Complete ===${NC}"
echo ""
echo "📚 Next steps:"
echo "  1. Review branch protection rules in .github/branch-protection.md"
echo "  2. Configure your IDE/editor (see .vscode/settings.json)"
echo "  3. Run 'just build' to verify everything works"
echo "  4. Read CONTRIBUTING.md for development guidelines"
echo ""
echo "🔧 Useful commands:"
echo "  just build       - Build the kernel"
echo "  just test        - Run tests"
echo "  just run         - Run in QEMU"
echo "  just debug       - Debug with GDB"
echo "  just clippy      - Run clippy checks"
echo "  just fmt         - Format code"
echo ""
echo "Happy hacking! 🚀"