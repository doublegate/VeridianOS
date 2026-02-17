# Software Porting Guide

This comprehensive guide covers porting existing Linux/POSIX software to VeridianOS. Despite being a microkernel OS with capability-based security, VeridianOS provides extensive POSIX compatibility to minimize porting effort while taking advantage of enhanced security features.

## Overview

### Porting Philosophy

VeridianOS takes a **pragmatic approach** to software compatibility:

1. **POSIX Compatibility Layer**: Full POSIX API implementation for existing software
2. **Capability Translation**: Automatic translation from POSIX permissions to capabilities
3. **Minimal Changes**: Most software ports with little to no modification
4. **Enhanced Security**: Ported software benefits from capability-based isolation
5. **Performance**: Native APIs available for performance-critical applications

### Architecture Compatibility

VeridianOS supports software for all target architectures:

| Architecture | Status | Target Triple |
|--------------|--------|---------------|
| **x86_64** | ✅ Full Support | `x86_64-veridian` |
| **AArch64** | ✅ Full Support | `aarch64-veridian` |
| **RISC-V** | ✅ Full Support | `riscv64gc-veridian` |

## Cross-Compilation Setup

### Toolchain Installation

Install the VeridianOS cross-compilation toolchain:

```bash
# Download pre-built toolchain (recommended)
curl -O https://releases.veridian-os.org/toolchain/veridian-toolchain-latest.tar.xz
sudo tar -xf veridian-toolchain-latest.tar.xz -C /opt/

# Add to PATH
export PATH="/opt/veridian-toolchain/bin:$PATH"

# Verify installation
x86_64-veridian-gcc --version
```

### Sysroot Configuration

Set up the target system root:

```bash
# Download VeridianOS sysroot
curl -O https://releases.veridian-os.org/sysroot/veridian-sysroot-latest.tar.xz
sudo mkdir -p /opt/veridian-sysroot
sudo tar -xf veridian-sysroot-latest.tar.xz -C /opt/veridian-sysroot/

# Set environment variables
export VERIDIAN_SYSROOT="/opt/veridian-sysroot"
export PKG_CONFIG_SYSROOT_DIR="$VERIDIAN_SYSROOT"
export PKG_CONFIG_PATH="$VERIDIAN_SYSROOT/usr/lib/pkgconfig"
```

### Build Environment

Configure your build environment for cross-compilation:

```bash
# Create build script
cat > build-for-veridian.sh << 'EOF'
#!/bin/bash
export CC="x86_64-veridian-gcc"
export CXX="x86_64-veridian-g++"
export AR="x86_64-veridian-ar"
export STRIP="x86_64-veridian-strip"
export RANLIB="x86_64-veridian-ranlib"

export CFLAGS="-O2 -pipe"
export CXXFLAGS="$CFLAGS"
export LDFLAGS="-static"  # Use static linking initially

exec "$@"
EOF
chmod +x build-for-veridian.sh
```

## POSIX Compatibility Layer

### Three-Layer Architecture

VeridianOS implements POSIX compatibility through a sophisticated layered approach:

```
┌─────────────────────────────────────────────────────────────┐
│                    POSIX Application                        │
├─────────────────────────────────────────────────────────────┤
│ POSIX API Layer      │ open(), read(), write(), socket()    │
├─────────────────────────────────────────────────────────────┤
│ Translation Layer    │ POSIX → Capability mapping          │
├─────────────────────────────────────────────────────────────┤
│ Native IPC Layer     │ Zero-copy, capability-protected IPC  │
└─────────────────────────────────────────────────────────────┘
```

### File System Operations

POSIX file operations are automatically translated to capability-based operations:

```c
// POSIX API (application code unchanged)
int fd = open("/etc/config", O_RDONLY);
char buffer[1024];
ssize_t bytes = read(fd, buffer, sizeof(buffer));
close(fd);

// Internal translation (transparent to application)
capability_t vfs_cap = veridian_get_capability("vfs");
capability_t file_cap = veridian_vfs_open(vfs_cap, "/etc/config", O_RDONLY);
ssize_t bytes = veridian_file_read(file_cap, buffer, sizeof(buffer));
veridian_capability_close(file_cap);
```

### Network Operations

Socket operations work transparently with automatic capability management:

```c
// Standard POSIX networking
int sock = socket(AF_INET, SOCK_STREAM, 0);
struct sockaddr_in addr = {
    .sin_family = AF_INET,
    .sin_port = htons(80),
    .sin_addr.s_addr = inet_addr("192.168.1.1")
};
connect(sock, (struct sockaddr*)&addr, sizeof(addr));

// Internally mapped to capability-based network access
capability_t net_cap = veridian_get_capability("network");
capability_t sock_cap = veridian_net_socket(net_cap, AF_INET, SOCK_STREAM, 0);
veridian_net_connect(sock_cap, &addr, sizeof(addr));
```

## Common Porting Scenarios

### System Utilities

Most UNIX utilities compile with minimal or no changes:

```bash
# Example: Porting GNU Coreutils
cd coreutils-9.4
./configure --host=x86_64-veridian \
           --prefix=/usr \
           --disable-nls \
           --enable-static-link
make -j$(nproc)
make DESTDIR=$VERIDIAN_SYSROOT install
```

**Success Rate**: ~95% of coreutils work without modification

### Text Editors and Development Tools

```bash
# Vim
cd vim-9.0
./configure --host=x86_64-veridian \
           --with-features=huge \
           --disable-gui \
           --enable-static-link
make -j$(nproc)

# GCC (as a cross-compiler)
cd gcc-13.2.0
mkdir build && cd build
../configure --target=x86_64-veridian \
           --prefix=/usr \
           --enable-languages=c,c++ \
           --disable-multilib
make -j$(nproc)
```

### Network Applications

```bash
# cURL
cd curl-8.4.0
./configure --host=x86_64-veridian \
           --prefix=/usr \
           --with-ssl \
           --disable-shared \
           --enable-static
make -j$(nproc)

# OpenSSH
cd openssh-9.5p1
./configure --host=x86_64-veridian \
           --prefix=/usr \
           --disable-strip \
           --with-sandbox=no
make -j$(nproc)
```

### Programming Language Interpreters

#### Python

```bash
cd Python-3.12.0
./configure --host=x86_64-veridian \
           --build=x86_64-linux-gnu \
           --prefix=/usr \
           --disable-shared \
           --with-system-ffi=no \
           ac_cv_file__dev_ptmx=no \
           ac_cv_file__dev_ptc=no \
           ac_cv_working_tzset=yes
make -j$(nproc)
```

#### Node.js

```bash
cd node-v20.9.0
./configure --dest-cpu=x64 \
           --dest-os=veridian \
           --cross-compiling \
           --without-npm
make -j$(nproc)
```

#### Go Compiler

```bash
cd go1.21.3/src
GOOS=veridian GOARCH=amd64 ./make.bash
```

### Databases

```bash
# SQLite
cd sqlite-autoconf-3430200
./configure --host=x86_64-veridian \
           --prefix=/usr \
           --enable-static \
           --disable-shared
make -j$(nproc)

# PostgreSQL (client libraries)
cd postgresql-16.0
./configure --host=x86_64-veridian \
           --prefix=/usr \
           --without-readline \
           --disable-shared
make -C src/interfaces/libpq -j$(nproc)
```

## VeridianOS-Specific Adaptations

### Process Creation

VeridianOS doesn't support `fork()` for security reasons. Use `posix_spawn()` instead:

```c
// Traditional approach (not supported)
#if 0
pid_t pid = fork();
if (pid == 0) {
    execve(program, argv, envp);
    _exit(1);
} else if (pid > 0) {
    waitpid(pid, &status, 0);
}
#endif

// VeridianOS approach
pid_t pid;
posix_spawnattr_t attr;
posix_spawnattr_init(&attr);

int result = posix_spawn(&pid, program, NULL, &attr, argv, envp);
if (result == 0) {
    waitpid(pid, &status, 0);
}
posix_spawnattr_destroy(&attr);
```

### Memory Management

VeridianOS provides enhanced memory management with capability-based access:

```c
// Standard POSIX (works unchanged)
void *ptr = mmap(NULL, size, PROT_READ | PROT_WRITE, 
                MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

// Enhanced VeridianOS API (optional, for better performance)
capability_t mem_cap = veridian_get_capability("memory");
void *ptr = veridian_mmap(mem_cap, NULL, size, 
                         VERIDIAN_PROT_READ | VERIDIAN_PROT_WRITE,
                         VERIDIAN_MAP_PRIVATE);
```

### Signal Handling

Signals work through a user-space signal daemon:

```c
// Standard signal handling (works with slight latency)
void signal_handler(int sig) {
    printf("Received signal %d\n", sig);
}

signal(SIGINT, signal_handler);  // Works via signal daemon
sigaction(SIGTERM, &action, NULL);  // Preferred for precise control

// VeridianOS async notification (optional, for low latency)
veridian_async_notify_t notify;
veridian_async_notify_init(&notify, VERIDIAN_NOTIFY_INTERRUPT);
veridian_async_notify_register(&notify, interrupt_handler);
```

### Device Access

Device access requires capabilities but POSIX APIs work transparently:

```c
// Standard POSIX (automatic capability management)
int fd = open("/dev/ttyS0", O_RDWR);
write(fd, "Hello", 5);

// Native VeridianOS (explicit capability management)
capability_t serial_cap = veridian_request_capability("serial.ttyS0");
veridian_device_write(serial_cap, "Hello", 5);
```

## Build System Integration

### Autotools Support

Create a cache file for autotools projects:

```bash
# veridian-config.cache
ac_cv_func_fork=no
ac_cv_func_fork_works=no
ac_cv_func_vfork=no
ac_cv_func_vfork_works=no
ac_cv_func_epoll_create=no
ac_cv_func_epoll_ctl=no
ac_cv_func_epoll_wait=no
ac_cv_func_kqueue=no
ac_cv_func_sendfile=no
ac_cv_header_sys_epoll_h=no
ac_cv_header_sys_event_h=no
ac_cv_working_fork=no
ac_cv_working_vfork=no
```

Update `config.sub` to recognize VeridianOS:

```bash
# Add to config.sub after other OS patterns
*-veridian*)
    os=-veridian
    ;;
```

### CMake Support

Create `VeridianOSToolchain.cmake`:

```cmake
set(CMAKE_SYSTEM_NAME VeridianOS)
set(CMAKE_SYSTEM_VERSION 1.0)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(CMAKE_C_COMPILER x86_64-veridian-gcc)
set(CMAKE_CXX_COMPILER x86_64-veridian-g++)
set(CMAKE_ASM_COMPILER x86_64-veridian-gcc)

set(CMAKE_FIND_ROOT_PATH ${VERIDIAN_SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

# VeridianOS-specific compile flags
set(CMAKE_C_FLAGS_INIT "-static")
set(CMAKE_CXX_FLAGS_INIT "-static")

# Disable tests that won't work in cross-compilation
set(CMAKE_CROSSCOMPILING_EMULATOR "")
```

Use with: `cmake -DCMAKE_TOOLCHAIN_FILE=VeridianOSToolchain.cmake`

### Meson Support

Create `veridian-cross.txt`:

```ini
[binaries]
c = 'x86_64-veridian-gcc'
cpp = 'x86_64-veridian-g++'
ar = 'x86_64-veridian-ar'
strip = 'x86_64-veridian-strip'
pkgconfig = 'x86_64-veridian-pkg-config'

[host_machine]
system = 'veridian'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'

[properties]
sys_root = '/opt/veridian-sysroot'
```

Use with: `meson setup builddir --cross-file veridian-cross.txt`

## Advanced Porting Techniques

### Conditional Compilation

Use preprocessor macros for VeridianOS-specific code:

```c
#ifdef __VERIDIAN__
    // VeridianOS-specific implementation
    capability_t cap = veridian_get_capability("network");
    result = veridian_net_operation(cap, data);
#else
    // Standard POSIX implementation
    result = standard_operation(data);
#endif
```

### Runtime Feature Detection

Detect VeridianOS features at runtime:

```c
int has_veridian_features(void) {
    return access("/proc/veridian", F_OK) == 0;
}

void optimized_operation(void) {
    if (has_veridian_features()) {
        // Use VeridianOS-optimized path
        veridian_zero_copy_operation();
    } else {
        // Fallback to standard implementation
        standard_operation();
    }
}
```

### Library Compatibility

Create wrapper libraries for complex dependencies:

```c
// libcompat-veridian.c - Compatibility layer
#include <errno.h>

// Stub out unavailable functions
int epoll_create(int size) {
    errno = ENOSYS;
    return -1;
}

int inotify_init(void) {
    errno = ENOSYS;
    return -1;
}

// Provide alternatives using VeridianOS APIs
int veridian_poll(struct pollfd *fds, nfds_t nfds, int timeout) {
    // Implement using VeridianOS async notification
    return -1;  // Placeholder
}
```

## Performance Optimization

### Zero-Copy Operations

Take advantage of VeridianOS zero-copy capabilities:

```c
// Standard approach (copy-based)
char buffer[8192];
ssize_t bytes = read(fd, buffer, sizeof(buffer));
write(output_fd, buffer, bytes);

// VeridianOS zero-copy (when both fds support it)
if (veridian_supports_zero_copy(fd, output_fd)) {
    veridian_zero_copy_transfer(fd, output_fd, bytes);
} else {
    // Fallback to standard approach
}
```

### Async I/O

Use VeridianOS async I/O for better performance:

```c
// Traditional blocking I/O
for (int i = 0; i < num_files; i++) {
    process_file(files[i]);
}

// VeridianOS async I/O
veridian_async_context_t ctx;
veridian_async_init(&ctx);

for (int i = 0; i < num_files; i++) {
    veridian_async_submit(&ctx, process_file_async, files[i]);
}

veridian_async_wait_all(&ctx);
```

### Capability Caching

Cache capabilities for frequently accessed resources:

```c
static capability_t cached_vfs_cap = VERIDIAN_INVALID_CAPABILITY;

capability_t get_vfs_capability(void) {
    if (cached_vfs_cap == VERIDIAN_INVALID_CAPABILITY) {
        cached_vfs_cap = veridian_get_capability("vfs");
    }
    return cached_vfs_cap;
}
```

## Testing and Validation

### Basic Functionality Testing

```bash
# Test basic operation
./ported-application --version
./ported-application --help

# Test with sample data
echo "test input" | ./ported-application
./ported-application < test-input.txt > test-output.txt
```

### Stress Testing

```bash
# Test concurrent operation
for i in {1..10}; do
    ./ported-application &
done
wait

# Test memory usage
./ported-application &
PID=$!
while kill -0 $PID 2>/dev/null; do
    ps -o pid,vsz,rss $PID
    sleep 1
done
```

### Capability Verification

```bash
# Verify capability usage
veridian-capability-trace ./ported-application
# Should show only necessary capabilities are requested

# Test with restricted capabilities
veridian-sandbox --capabilities=minimal ./ported-application
```

## Packaging and Distribution

### Port Recipes

Create standardized port recipes for the VeridianOS package system:

```toml
# ports/editors/vim/port.toml
[package]
name = "vim"
version = "9.0"
description = "Vi IMproved text editor"
source = "https://github.com/vim/vim/archive/v9.0.tar.gz"
sha256 = "..."

[build]
system = "autotools"
configure_args = [
    "--host=x86_64-veridian",
    "--with-features=huge",
    "--disable-gui",
    "--enable-static-link"
]

[dependencies]
build = ["gcc", "make", "ncurses-dev"]
runtime = ["ncurses"]

[capabilities]
required = ["vfs:read,write", "terminal:access"]
optional = ["network:connect"]  # For plugin downloads

[patches]
files = ["vim-veridian.patch", "disable-fork.patch"]
```

### Package Metadata

Include VeridianOS-specific metadata:

```yaml
# .veridian-package.yaml
name: vim
version: 9.0-veridian1
architecture: [x86_64, aarch64, riscv64]
categories: [editor, development]

capabilities:
  required:
    - vfs:read,write
    - terminal:access
  optional:
    - network:connect

compatibility:
  posix_compliance: 95%
  veridian_native: false
  zero_copy_io: false

performance:
  startup_time: "< 100ms"
  memory_usage: "< 10MB"
```

## Troubleshooting

### Common Issues

**1. Undefined References**
```bash
# Problem: undefined reference to `fork`
# Solution: Use posix_spawn or disable fork-dependent features
CFLAGS="-DNO_FORK" ./configure --host=x86_64-veridian
```

**2. Missing Headers**
```bash
# Problem: sys/epoll.h: No such file or directory
# Solution: Use select() or poll() instead, or disable feature
CFLAGS="-DNO_EPOLL" ./configure
```

**3. Runtime Capability Errors**
```bash
# Problem: Permission denied accessing /dev/random
# Solution: Request entropy capability
veridian-capability-request entropy ./application
```

### Debugging Techniques

```bash
# Check for undefined symbols
x86_64-veridian-nm -u binary | grep -v "^ *U _"

# Verify library dependencies
x86_64-veridian-ldd binary

# Trace system calls during execution
veridian-strace ./binary

# Monitor capability usage
veridian-capability-monitor ./binary
```

### Performance Analysis

```bash
# Profile application performance
veridian-perf record ./binary
veridian-perf report

# Analyze IPC usage
veridian-ipc-trace ./binary

# Monitor memory allocation
veridian-malloc-trace ./binary
```

## Contributing Ports

### Submission Process

1. **Create Port Recipe**: Follow the template format
2. **Test Thoroughly**: Ensure functionality and performance
3. **Document Changes**: Explain any VeridianOS-specific modifications
4. **Submit Pull Request**: To the VeridianOS ports repository

### Quality Guidelines

- **Minimal Patches**: Prefer runtime detection over compile-time patches
- **Performance**: Measure and optimize for VeridianOS features
- **Security**: Verify capability usage is minimal and appropriate
- **Documentation**: Include usage examples and troubleshooting

## Future Enhancements

### Planned Improvements

**Phase 5: Enhanced Compatibility**
- Dynamic linking support
- Container compatibility layer
- Graphics acceleration APIs

**Phase 6: Native Integration**
- VeridianOS-native GUI toolkit
- Zero-copy graphics pipeline
- Hardware acceleration APIs

### Research Areas

1. **Automatic Port Generation**: AI-assisted porting from source analysis
2. **Binary Translation**: Run Linux binaries directly with capability translation
3. **Just-in-Time Capabilities**: Dynamic capability request during execution

This comprehensive porting guide enables developers to bring existing software to VeridianOS while taking advantage of its enhanced security and performance features.