# Developer helpers

Small tools used while working on VeridianOS. CI does not run them; the build and test entry
points they call (`build-kernel.sh`, `scripts/boot-test.sh`, `scripts/run-rootfs-tests.sh`,
`tools/cross/`) are the real interfaces. Logs go to `target/dev-logs/` unless `LOG_DIR` says
otherwise.

| Script | What it does |
|---|---|
| `gate.sh [--boot] [--rootfs]` | The pre-commit gate: fmt, clippy `-D warnings` on the three bare-metal targets and the host library, host unit tests; `--boot` adds the tri-arch build and QEMU boot tests, `--rootfs` the x86_64 runtime suite. Non-zero exit if any step fails. |
| `runtime-tests.sh` | Rebuild the dev kernel, the rootfs staging tree (which compiles the runtime test programs) and the BlockFS image, then run `scripts/run-rootfs-tests.sh` with the musl checks required. |
| `guest.py ARCH LOG CMD...` | Boot the AArch64 or RISC-V musl root (`target/rootfs-musl-ARCH.img`, snapshot mode) and type each command at the shell, waiting for the prompt after each; the serial output goes to LOG. `SMP`, `RISCV_CPU` and `PROMPT_TIMEOUT` adjust the run. |
| `rootfs-monitor.sh` | The same suite under QEMU with an HMP monitor socket, QEMU left running afterwards, for inspecting a stall (`info registers`, then `addr2line` on the kernel ELF). |
| `lint-cross.sh` | Lint the changed `tools/cross` files: `bash -n`, shellcheck, ruff, and `PATCHES.md` up to date. |
| `mkpatch.py SRC OUT HEADER EDITS.json` | Build a `-p1` patch from exact, once-only text replacements in a source tree (for `tools/cross/*-patches/`) and check it with `patch --dry-run`. |
| `pkg-cc.sh SRC BLD FILE REL` | Compile an edited copy of a package source file with the exact flags its build used (compile_commands.json, CMake `flags.make`, or an Automake dry run), syntax only, and print the diagnostics. |
| `warn-triage/scan.py LOG OUT.json`, `analyze.py OUT.json` | Inventory every warning in a KDE cross-build log by kind, phase and package, then group the compiler warnings by flag and origin. |
