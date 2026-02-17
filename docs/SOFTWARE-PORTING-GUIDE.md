# VeridianOS Software Porting Guide

## Overview

This guide provides comprehensive instructions for porting existing Linux/POSIX software to VeridianOS. As a new microkernel OS with capability-based security, VeridianOS requires special considerations when porting software, but our POSIX compatibility layer aims to minimize required changes.

## Target Triple and Build Configuration

### Official Target Triples

VeridianOS uses the following target triple format:
```
<arch>-veridian-<subsystem>-<abi>
```

Standard triples:
- `x86_64-veridian` (general userland)
- `aarch64-veridian`
- `riscv64gc-veridian`

Specialized triples:
- `x86_64-veridian-kernel-none` (kernel code)
- `aarch64-veridian-driver-elf` (driver development)

## Cross-Compilation Setup

### 1. Building the Cross-Toolchain

First, set up a cross-compilation environment on your host system:

```bash
# Install dependencies (Ubuntu/Debian)
sudo apt-get install build-essential texinfo bison flex

# Build binutils for VeridianOS
wget https://ftp.gnu.org/gnu/binutils/binutils-2.42.tar.gz
tar xzf binutils-2.42.tar.gz
cd binutils-2.42
mkdir build && cd build
../configure --target=x86_64-veridian \
             --prefix=/opt/veridian-toolchain \
             --with-sysroot=/opt/veridian-sysroot \
             --disable-nls
make -j$(nproc)
sudo make install
```

### 2. Building Cross-GCC

```bash
# Download GCC
wget https://ftp.gnu.org/gnu/gcc/gcc-13.2.0/gcc-13.2.0.tar.gz
tar xzf gcc-13.2.0.tar.gz
cd gcc-13.2.0

# Download prerequisites
./contrib/download_prerequisites

# Configure and build
mkdir build && cd build
../configure --target=x86_64-veridian \
             --prefix=/opt/veridian-toolchain \
             --with-sysroot=/opt/veridian-sysroot \
             --enable-languages=c,c++ \
             --disable-multilib \
             --disable-libssp \
             --disable-libquadmath \
             --disable-libgomp \
             --with-newlib
make -j$(nproc) all-gcc
sudo make install-gcc
```

### 3. Installing VeridianOS Headers and Libraries

Before compiling software, you need VeridianOS headers and libraries in your sysroot:

```bash
# Create sysroot structure
sudo mkdir -p /opt/veridian-sysroot/{usr/{include,lib},lib,etc}

# Copy VeridianOS headers (from VeridianOS source)
sudo cp -r $VERIDIAN_SRC/libs/veridian-abi/include/* /opt/veridian-sysroot/usr/include/
sudo cp -r $VERIDIAN_SRC/libs/veridian-libc/include/* /opt/veridian-sysroot/usr/include/

# Copy libraries (after building libc)
sudo cp $VERIDIAN_BUILD/libs/veridian-libc/libc.a /opt/veridian-sysroot/usr/lib/
sudo cp $VERIDIAN_BUILD/libs/veridian-libc/crt*.o /opt/veridian-sysroot/usr/lib/
```

## POSIX Compatibility Layer

### Three-Layer Architecture

VeridianOS implements POSIX compatibility through three layers:

1. **POSIX API Layer**: Standard POSIX functions (open, read, write, etc.)
2. **Translation Layer**: Converts POSIX operations to capability-based operations
3. **Native IPC Layer**: VeridianOS's zero-copy, capability-protected IPC

### File Descriptor Translation

POSIX file descriptors are translated to capability handles:

```c
// Internal translation table in libc
typedef struct {
    int posix_fd;
    capability_t veridian_cap;
    int flags;
} fd_translation_t;

// Example: open() implementation
int open(const char *path, int flags, ...) {
    // Get VFS capability
    capability_t vfs_cap = __veridian_get_vfs_capability();
    
    // Send IPC to VFS service
    struct vfs_open_request req = {
        .path = path,
        .flags = translate_posix_flags(flags)
    };
    
    capability_t file_cap;
    int result = __veridian_ipc_call(vfs_cap, &req, &file_cap);
    
    if (result < 0) {
        errno = -result;
        return -1;
    }
    
    // Allocate POSIX fd and map to capability
    int fd = __allocate_fd();
    __map_fd_to_capability(fd, file_cap);
    
    return fd;
}
```

## Common Porting Scenarios

### 1. Basic UNIX Utilities (coreutils, busybox)

Most basic utilities compile with minimal changes:

```bash
# Example: Compiling GNU coreutils
./configure --host=x86_64-veridian \
            --prefix=/usr \
            --disable-nls \
            CC=x86_64-veridian-gcc

make -j$(nproc)
make DESTDIR=$VERIDIAN_SYSROOT install
```

### 2. Network Applications

Network applications require the socket API implementation:

```bash
# Example: Compiling curl
./configure --host=x86_64-veridian \
            --prefix=/usr \
            --with-ssl=no \
            --disable-shared \
            CC=x86_64-veridian-gcc

make -j$(nproc)
```

### 3. Build Systems and Interpreters

#### Python
```bash
# Cross-compiling Python requires special handling
./configure --host=x86_64-veridian \
            --build=x86_64-linux-gnu \
            --prefix=/usr \
            --enable-shared=no \
            --with-system-ffi=no \
            ac_cv_file__dev_ptmx=no \
            ac_cv_file__dev_ptc=no

make -j$(nproc)
```

## Handling VeridianOS-Specific Features

### 1. Process Creation (No fork())

VeridianOS doesn't support fork() for security reasons. Use posix_spawn():

```c
// Instead of:
pid_t pid = fork();
if (pid == 0) {
    execve(path, argv, envp);
}

// Use:
pid_t pid;
posix_spawn(&pid, path, NULL, NULL, argv, envp);
```

### 2. Capability-Based Permissions

Some operations require explicit capabilities:

```c
// Traditional UNIX approach
int fd = open("/dev/gpu0", O_RDWR);
void *mmio = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);

// VeridianOS approach (if using native API)
capability_t gpu_cap = veridian_request_capability("gpu.device.0");
void *mmio = veridian_map_mmio(gpu_cap, size);
```

### 3. Signal Handling Differences

VeridianOS implements signals through a user-space daemon:

```c
// Signal handling works but with slight latency
signal(SIGINT, handler);  // Works via signal daemon
sigaction(...);           // Preferred for better control
```

## Autoconf/Automake Support

### Adding VeridianOS to config.sub

Edit `config.sub` in autotools projects:

```bash
# Add after other OS patterns
-veridian*)
    os=-veridian
    ;;
```

### Configure Cache Variables

Create a cache file for common configure tests:

```bash
# veridian-config.cache
ac_cv_func_fork=no
ac_cv_func_fork_works=no
ac_cv_func_vfork=no
ac_cv_func_vfork_works=no
ac_cv_header_sys_epoll_h=no
ac_cv_func_epoll_create=no
```

Use with: `./configure --cache-file=veridian-config.cache`

## CMake Toolchain File

Create `VeridianToolchain.cmake`:

```cmake
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(CMAKE_C_COMPILER x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)

set(CMAKE_FIND_ROOT_PATH /opt/veridian-sysroot)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

# VeridianOS-specific flags
set(CMAKE_C_FLAGS_INIT "-static")
set(CMAKE_CXX_FLAGS_INIT "-static")
```

## Static vs Dynamic Linking

### Phase 2-3: Static Linking Only

Initially, use static linking for all applications:

```bash
LDFLAGS="-static" ./configure ...
```

Benefits:
- No dynamic linker complexity
- Self-contained binaries
- Easier debugging

### Phase 4+: Dynamic Linking Support

When dynamic linking is available:
- Shared libraries in `/usr/lib`
- Dynamic linker at `/lib/ld-veridian.so.1`
- RPATH support for custom library paths

## Common Porting Issues and Solutions

### 1. Missing System Calls

```c
// Stub out unavailable syscalls
#ifdef __veridian__
int epoll_create(int size) {
    errno = ENOSYS;
    return -1;
}
#endif
```

### 2. /proc Filesystem Dependencies

```c
// Provide alternatives to /proc
#ifdef __veridian__
    // Use sysctl or capability-based API
    veridian_get_process_info(pid, &info);
#else
    // Traditional /proc reading
    snprintf(path, sizeof(path), "/proc/%d/stat", pid);
#endif
```

### 3. Thread-Local Storage

VeridianOS supports TLS through:
- Static TLS for initial libraries
- Dynamic TLS for dlopen'd libraries
- Architecture-specific registers (fs/gs on x86_64)

## Testing Ported Software

### 1. Basic Functionality Test

```bash
# In VeridianOS QEMU environment
$ /usr/bin/ls -la
$ /usr/bin/echo "Hello VeridianOS"
```

### 2. Stress Testing

```bash
# Test process creation
$ for i in {1..100}; do /usr/bin/true & done

# Test file operations
$ /usr/bin/dd if=/dev/zero of=test bs=1M count=10
```

### 3. Integration Testing

Create test scripts that exercise ported software with VeridianOS-specific features.

## Contributing Ports

### Port Submission Guidelines

1. Create a port recipe in `ports/<category>/<name>/`
2. Include:
   - `build.toml` - Build configuration
   - `patches/` - VeridianOS-specific patches
   - `files/` - Additional files needed

### Example Port Recipe

```toml
# ports/shells/bash/build.toml
[package]
name = "bash"
version = "5.2"
source = "https://ftp.gnu.org/gnu/bash/bash-5.2.tar.gz"
sha256 = "..."

[build]
configure_args = [
    "--host=x86_64-veridian",
    "--prefix=/usr",
    "--disable-nls",
    "--enable-static-link"
]

[patches]
files = ["veridian-config.patch", "no-fork-support.patch"]

[dependencies]
build = ["gcc", "make"]
runtime = ["libc", "ncurses"]
```

## Advanced Topics

### 1. Graphics Applications

For GUI applications in Phase 6:
- Wayland client library port
- Mesa for OpenGL support
- Input device handling via capabilities

### 2. Language Runtimes

- **JVM**: Port OpenJDK with VeridianOS threading
- **Node.js**: V8 with custom memory allocator
- **Ruby/Perl**: Standard interpreter ports

### 3. Virtualization

For container/VM support:
- Namespace emulation via capabilities
- cgroup-like resource limits
- OCI runtime compatibility layer

## Troubleshooting

### Common Build Errors

1. **"Unknown target system"**
   - Update config.sub/config.guess
   - Use --host explicitly

2. **"Function not implemented"**
   - Check for Linux-specific syscalls
   - Link against VeridianOS compatibility library

3. **"Cannot find -lpthread"**
   - Ensure VeridianOS libc is in sysroot
   - Use -static flag if dynamic linking unavailable

### Debug Techniques

```bash
# Check undefined symbols
x86_64-veridian-nm -u binary

# Verify linking
x86_64-veridian-ldd binary

# Test in QEMU
qemu-system-x86_64 -kernel veridian.elf -initrd initrd.img \
    -append "init=/usr/bin/your-ported-app"
```

## Resources

- VeridianOS Porting Examples: `/examples/ports/`
- POSIX Compliance Matrix: `/docs/POSIX-COMPLIANCE.md`
- Capability API Reference: `/docs/api/capabilities.md`
- Community Port Repository: https://github.com/veridian-ports

---

For questions or assistance with porting, contact the VeridianOS development team or join our community forums.