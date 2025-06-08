# Branch Protection Rules for VeridianOS

This document outlines the recommended branch protection rules for the VeridianOS repository.

## Protected Branches

### `main` Branch

The `main` branch is the primary branch and should have the strictest protection.

**Protection Rules:**

- ✅ **Require pull request reviews before merging**
  - Required approving reviews: 2
  - Dismiss stale pull request approvals when new commits are pushed
  - Require review from CODEOWNERS
  - Restrict who can dismiss pull request reviews

- ✅ **Require status checks to pass before merging**
  - Require branches to be up to date before merging
  - Required status checks:
    - `CI / Quick Checks`
    - `CI / Build & Test (x86_64-veridian)`
    - `CI / Build & Test (aarch64-veridian)`
    - `CI / Build & Test (riscv64gc-veridian)`
    - `CI / Documentation`
    - `CI / Security Audit`
    - `CI / CI Summary`

- ✅ **Require conversation resolution before merging**

- ✅ **Require signed commits**

- ✅ **Include administrators**

- ✅ **Restrict who can push to matching branches**
  - Restrict pushes that create matching branches
  - Allow specified actors to bypass (for releases only)

- ❌ **Do not allow force pushes**

- ❌ **Do not allow deletions**

### `develop` Branch

The `develop` branch is for integration of features before release.

**Protection Rules:**

- ✅ **Require pull request reviews before merging**
  - Required approving reviews: 1
  - Dismiss stale pull request approvals when new commits are pushed

- ✅ **Require status checks to pass before merging**
  - Required status checks:
    - `CI / Quick Checks`
    - `CI / Build & Test (x86_64-veridian)`
    - `CI / CI Summary`

- ✅ **Require conversation resolution before merging**

- ❌ **Do not allow force pushes**

- ❌ **Do not allow deletions**

## Branch Naming Convention

Use the following branch naming patterns:

- `feature/*` - New features
- `bugfix/*` - Bug fixes
- `hotfix/*` - Urgent fixes for production
- `release/*` - Release preparation branches
- `chore/*` - Maintenance tasks
- `docs/*` - Documentation updates
- `test/*` - Test additions or fixes
- `refactor/*` - Code refactoring

## Automated Rules

### Stale PR Management

Configure GitHub Actions to:
- Label PRs as "stale" after 30 days of inactivity
- Close stale PRs after 60 days of inactivity
- Exempt PRs with labels: `work-in-progress`, `blocked`, `help-wanted`

### Auto-merge for Dependabot

Allow auto-merge for Dependabot PRs that:
- Pass all required checks
- Are patch or minor version updates
- Have no security vulnerabilities

## Setting Up Branch Protection

To configure these rules in GitHub:

1. Go to Settings → Branches
2. Add rule for `main` and `develop`
3. Configure protection rules as specified above
4. Save changes

## CODEOWNERS

Create a `.github/CODEOWNERS` file to specify code ownership:

```
# Global owners
* @doublegate

# Kernel core
/kernel/src/main.rs @doublegate
/kernel/src/arch/ @doublegate

# Memory management
/kernel/src/mm/ @doublegate

# Documentation
/docs/ @doublegate
*.md @doublegate
```

## Merge Strategies

- **main**: Squash and merge (clean history)
- **develop**: Create a merge commit (preserve feature history)
- **Feature branches**: Delete after merge

## Emergency Procedures

In case of critical issues:

1. Create a `hotfix/*` branch from `main`
2. Fix the issue with minimal changes
3. Create PR with "HOTFIX" label
4. Requires only 1 approval for emergency merge
5. Must still pass all CI checks

## Review Guidelines

Reviewers should check:

1. Code follows Rust best practices
2. Tests are included for new functionality
3. Documentation is updated
4. No security vulnerabilities introduced
5. Performance impact is acceptable
6. Architecture decisions are sound

## Enforcement

These rules are enforced through:
- GitHub branch protection settings
- Git hooks (pre-commit, pre-push)
- CI/CD pipeline checks
- Code review process