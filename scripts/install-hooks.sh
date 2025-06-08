#!/usr/bin/env bash
# Install git hooks for VeridianOS development

set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}Installing VeridianOS git hooks...${NC}"

# Get the repository root
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || echo ".")

# Configure git to use our hooks directory
git config core.hooksPath .githooks

echo -e "${GREEN}✓ Git configured to use .githooks directory${NC}"

# Make sure all hooks are executable
for hook in "$REPO_ROOT"/.githooks/*; do
    if [ -f "$hook" ]; then
        chmod +x "$hook"
        echo -e "${GREEN}✓ Made $(basename "$hook") executable${NC}"
    fi
done

# Optional: Copy hooks to .git/hooks for compatibility
if [ "${COPY_TO_GIT_HOOKS:-0}" = "1" ]; then
    echo -e "${YELLOW}Copying hooks to .git/hooks...${NC}"
    cp -f "$REPO_ROOT"/.githooks/* "$REPO_ROOT"/.git/hooks/
fi

echo ""
echo -e "${GREEN}✅ Git hooks installed successfully!${NC}"
echo ""
echo "Hooks installed:"
echo "  • pre-commit  - Formatting and code quality checks"
echo "  • commit-msg  - Commit message format validation"
echo "  • pre-push    - Test execution before pushing"
echo ""
echo "Configuration options:"
echo "  • RUN_CLIPPY_PRECOMMIT=1  - Enable clippy in pre-commit (slow)"
echo "  • RUN_TESTS_PREPUSH=0     - Disable tests in pre-push"
echo ""
echo "To bypass hooks temporarily, use --no-verify flag:"
echo "  git commit --no-verify"
echo "  git push --no-verify"
echo ""