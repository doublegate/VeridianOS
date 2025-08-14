## Project Overview

This directory contains the source code for VeridianOS, a modern microkernel operating system written in Rust. The project's goal is to create a secure, modular, and high-performance OS suitable for a wide range of applications.

VeridianOS features a capability-based security model, zero-copy IPC, and supports multiple architectures, including x86_64, AArch64, and RISC-V. The kernel is designed to be minimal, with most services implemented as user-space processes.

The project is under active development and is organized into a series of phases, with the initial phases focused on establishing the core kernel features.

## Building and Running

The project uses `cargo` for building and `just` as a command runner to simplify common tasks.

**Key Commands:**

*   **Install dependencies:**
    ```bash
    ./scripts/install-deps.sh
    ```

*   **Build the kernel for all architectures:**
    ```bash
    ./build-kernel.sh all dev
    ```

*   **Build for a specific architecture (e.g., x86_64):**
    ```bash
    ./build-kernel.sh x86_64 dev
    ```

*   **Run the kernel in QEMU (x86_64):**
    ```bash
    just run
    ```

*   **Run tests:**
    ```bash
    just test
    ```

*   **Run benchmarks:**
    ```bash
    just bench
    ```

*   **Format the code:**
    ```bash
    just fmt
    ```

*   **Run the linter:**
    ```bash
    just clippy
    ```

## Development Conventions

*   **Code Style:** The project follows the standard Rust formatting guidelines, enforced by `rustfmt`.
*   **Linting:** `clippy` is used to catch common mistakes and improve code quality. All warnings are treated as errors.
*   **Testing:** The project has a custom `no_std` testing framework. Integration tests are located in the `kernel/tests` directory and are run using `just test`.
*   **Contributions:** The `CONTRIBUTING.md` file outlines the process for contributing to the project, including the development workflow and pull request process.
*   **Documentation:** The `docs/` directory contains extensive documentation on the project's architecture, development guide, and API reference.
