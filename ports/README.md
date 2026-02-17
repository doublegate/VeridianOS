# VeridianOS Ports Collection -- Bootstrap Packages

This directory contains Portfile.toml definitions for the bootstrap toolchain
and core development tools needed to build software for VeridianOS.

## Portfile Format

Each port lives in its own directory under `ports/<name>/` and is described by
a single `Portfile.toml`.  The file is parsed by the kernel's minimal TOML
parser (`kernel/src/pkg/toml_parser.rs`) and consumed by the port manager
(`kernel/src/pkg/ports/mod.rs`).

A Portfile contains four sections:

### [port] -- Package metadata

| Key          | Type   | Required | Description                              |
|--------------|--------|----------|------------------------------------------|
| `name`       | string | yes      | Port name (must match directory name)    |
| `version`    | string | yes      | Upstream version string (e.g. `"2.43"`) |
| `description`| string | no       | One-line summary                         |
| `homepage`   | string | no       | Project URL                              |
| `license`    | string | no       | SPDX license identifier                  |
| `category`   | string | no       | Port category (default: `"misc"`)       |
| `build_type` | string | no       | `autotools`, `cmake`, `meson`, `cargo`, `make`, or `custom` |

### [sources] -- Source archives

| Key        | Type          | Description                                    |
|------------|---------------|------------------------------------------------|
| `urls`     | string array  | Download URLs for source tarballs              |
| `checksums`| string array  | SHA-256 hex digests, one per URL (same order)  |

### [dependencies] -- Build and runtime dependencies

| Key       | Type         | Description                                 |
|-----------|--------------|---------------------------------------------|
| `build`   | string array | Ports required at build time                |
| `runtime` | string array | Ports required at run time (optional)       |

Dependencies are resolved transitively by `PortManager::resolve_build_deps()`
with cycle detection.

### [build] -- Build commands

| Key    | Type         | Description                                          |
|--------|--------------|------------------------------------------------------|
| `steps`| string array | Shell commands executed sequentially in the build dir |

If `steps` is omitted the port manager falls back to the default
configure/build/install sequence for the declared `build_type`.

The following environment variables are set automatically during the build:

| Variable       | Value                                       |
|----------------|---------------------------------------------|
| `$PKG_DIR`     | Staging directory for installed files        |
| `$SRC_DIR`     | Extracted source directory                   |
| `$BUILD_DIR`   | Out-of-tree build directory                  |
| `$PORT_NAME`   | Port name from `[port]`                     |
| `$PORT_VERSION`| Port version from `[port]`                  |

## Bootstrap Build Order

The bootstrap toolchain must be built in dependency order.  A minimal
bootstrap sequence is:

```
1. binutils    (no dependencies)
2. gcc         (depends on: binutils)
3. make        (no dependencies -- can build in parallel with 1-2)
4. pkg-config  (no dependencies -- can build in parallel with 1-2)
5. cmake       (depends on: make)
6. meson       (no dependencies -- needs host python3)
7. llvm        (depends on: cmake)
8. gdb         (depends on: binutils)
```

Packages without inter-dependencies may be built in parallel.  The port
manager resolves the full transitive order automatically via
`PortManager::resolve_build_deps()`.

## How to Build a Port

### Using the port manager (kernel-side, future)

When the VeridianOS user-space is fully functional, ports are built through the
port manager:

```
# From the VeridianOS shell
pkg port build binutils
pkg port build gcc
```

The port manager downloads sources, verifies checksums, sets up an isolated
build environment, executes the build steps, and packages the result into a
`.vpkg` archive.

### Manual cross-compilation (host system)

During early development, ports are cross-compiled on the host:

```bash
# Set up the environment
export TARGET=x86_64-veridian
export SYSROOT=/opt/veridian-sysroot
export PREFIX=/usr

# Example: build binutils
cd ports/binutils
tar xf /path/to/binutils-2.43.tar.xz
mkdir build && cd build
../binutils-2.43/configure \
    --target=$TARGET \
    --prefix=$PREFIX \
    --with-sysroot=$SYSROOT \
    --disable-nls \
    --disable-werror
make -j$(nproc)
make install DESTDIR=$SYSROOT
```

See `docs/SOFTWARE-PORTING-GUIDE.md` for detailed cross-compilation
instructions including sysroot setup, toolchain files, and POSIX compatibility
notes.

## Checksums

All `checksums` entries are currently placeholder values (`0000...`).  Before
using a port in production, replace each placeholder with the real SHA-256
digest of the corresponding source archive:

```bash
sha256sum binutils-2.43.tar.xz
# paste the hex digest into Portfile.toml
```

The port manager logs a warning for zero checksums and skips verification, but
will reject a non-zero checksum that does not match the downloaded file.

## Patches

VeridianOS-specific patches for a port should be placed in a `patches/`
subdirectory alongside the Portfile:

```
ports/
  binutils/
    Portfile.toml
    patches/
      veridian-target.patch
      config-sub.patch
```

Patch application is not yet automated by the port manager.  Apply patches
manually or add `patch` commands to the `[build] steps` array.

## Contributing New Ports

1. Create a directory under `ports/<name>/`.
2. Write a `Portfile.toml` following the format above.
3. Test that the build steps work on a host system using cross-compilation.
4. Replace placeholder checksums with real SHA-256 digests.
5. Add any VeridianOS-specific patches to `patches/`.
6. Submit a pull request with the new port.

When choosing a category, use one of the standard categories defined in
`kernel/src/pkg/ports/collection.rs`:

| Category   | Purpose                                  |
|------------|------------------------------------------|
| `core`     | Essential system packages                |
| `devel`    | Development tools and libraries          |
| `libs`     | Shared and static libraries              |
| `net`      | Networking utilities and daemons         |
| `security` | Security tools and cryptographic software|
| `utils`    | General-purpose utilities                |

## References

- Port manager source: `kernel/src/pkg/ports/mod.rs`
- TOML parser: `kernel/src/pkg/toml_parser.rs`
- Port collection: `kernel/src/pkg/ports/collection.rs`
- Software porting guide: `docs/SOFTWARE-PORTING-GUIDE.md`
- Software porting (mdBook): `docs/book/src/advanced/software-porting.md`
