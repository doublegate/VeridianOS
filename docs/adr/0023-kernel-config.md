# ADR 0023: the kernel configuration VeridianOS reports

- Status: accepted
- Date: 2026-11-03
- Tracking: v0.28.0 order of work, item 3 (syscall groups), step K. It moved ahead of namespace
  steps H3 to H6 because LTP's namespace and time tests stop on "Cannot parse kernel .config"
  before they check anything.

## Context

Programs ask which features the kernel was built with by reading its configuration. LTP does so
in about 95 syscall tests (`.needs_kconfigs` and `tst_kconfig_check`), looking in order at
`$KCONFIG_PATH`, `/proc/config.gz`, `/lib/modules/<release>/build/.config`,
`/lib/modules/<release>/config`, `/boot/config-<release>` and `/lib/kernel/config-<release>`.
With none of them present, every such test is TBROK, including the ones whose feature works
(the UTS, IPC and time namespace tests of ADR 0022).

A configuration is a set of claims. If it says y for a feature the kernel lacks, a test that
would have been skipped (TCONF, as on a Linux kernel built without it) runs and fails instead,
which hides real regressions among expected failures.

## Decision

1. **One source, `abi/kconfig.map`.** Each line is an option, its value, the Linux system calls
   it provides (`needs=`), optionally the architectures it applies to (`arch=`), and why.
   `tools/compat/gen_kconfig.py` renders it, per architecture, in the format of Linux's `.config`
   (`CONFIG_X=y`, `# CONFIG_X is not set`).
2. **An option is y only when the kernel does what Linux built with it does.** A feature done in
   part stays unset until it is whole: CHECKPOINT_RESTORE is unset although kcmp works, because
   PR_SET_MM, `map_files` and `msg_next_id` do not. Options nothing asks about are left out (an
   absent option reads as unset). The namespace options follow ADR 0022's steps: UTS, IPC and
   time are y, mount, user, PID and network flip to y as H3 to H6 land.
3. **The claims are checked.** `gen_kconfig.py --check`, in `scripts/dev/gate.sh` and CI, fails
   when an option that is not n names a system call that the kernel does not implement (missing,
   or a stub, as `tools/compat/syscall_coverage.py` classifies them) or that Linux lacks on an
   architecture the line applies to.
4. **It lives at `/boot/config-<release>`.** The root filesystem builds (`build-musl-rootfs.sh`,
   `build-busybox-rootfs.sh`, `build-compat-rootfs.sh`) install it under the release that uname
   reports (`kernel/src/utils/uname.rs`, ADR 0021), so the name cannot drift from uname.
   `/proc/config.gz` was not chosen: LTP reads it through `popen("zcat ...")`, a shell and a
   decompressor started for every test, which costs seconds per test under TCG, and the kernel
   would need a gzip writer.

## Consequences

- LTP's configuration checks run: tests of a feature that is there run, and tests of one that is
  not are TCONF rather than TBROK. The expected lists change accordingly with the next full runs.
- IPC_NS is y although SYSVIPC and POSIX_MQUEUE are unset, a combination Linux's Kconfig does not
  allow; the namespace itself works (ADR 0022 H1), which is what the option reports.
- The file describes the kernel, not the image, yet the image carries it: an image built against
  one tree and booted with another kernel reports the first tree's claims. The images are built
  from the same tree as the kernel in every script here.
- Adding a feature now has one more step: flipping its option in `abi/kconfig.map`.
