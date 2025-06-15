# Book Location Update - June 10, 2025

## Summary
Moved `book.toml` from the project root to `docs/book/` for better organization and consistency.

## Changes Made

### 1. File Relocation
- **From**: `/book.toml` (project root)
- **To**: `/docs/book/book.toml`

### 2. Path Updates in book.toml
```toml
# Old paths
src = "docs/book/src"
build-dir = "target/book"
theme = "docs/book-theme"
additional-css = ["docs/book-theme/veridian.css"]

# New paths (relative to docs/book/)
src = "src"
build-dir = "book"
theme = "../book-theme"
additional-css = ["../book-theme/veridian.css"]
```

### 3. CI/CD Update
Updated `.github/workflows/ci.yml` to build from new location:
```yaml
# Old
mdbook build
cp -r target/book artifacts/docs/guide

# New
cd docs/book && mdbook build
cd ../..
cp -r docs/book/book artifacts/docs/guide
```

## Benefits
1. **Better Organization**: All book-related files are now in `docs/book/`
2. **Cleaner Root**: Reduces clutter in the project root directory
3. **Consistency**: Book source (`src/`) and output (`book/`) are siblings
4. **Convention**: Follows common mdBook project structure

## Testing
- Confirmed mdBook builds successfully from new location
- Output directory is `docs/book/book/`
- All paths resolve correctly
- Theme CSS loads properly

## Migration Notes
- No content changes were made
- Only configuration and build paths were updated
- CI/CD pipeline will work with new structure