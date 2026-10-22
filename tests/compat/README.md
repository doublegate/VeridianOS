# Conformance suites

VeridianOS runs three upstream conformance suites in QEMU, built for each
architecture with the musl cross toolchain:

| Suite | Upstream | What runs |
|---|---|---|
| `ltp-syscalls` | [LTP](https://github.com/linux-test-project/ltp) 20260930 | every entry of `runtest/syscalls` |
| `open-posix` | the Open POSIX Test Suite (part of LTP) | the runnable conformance and functional tests its own build compiles (not `stress/`); its compile-only tests are judged at build time |
| `libc-test` | [musl libc-test](https://repo.or.cz/libc-test.git) at `7b95dfa5` | every test program its Makefile builds, static and dynamic; `src/api` is compile-only |

`expected-<arch>.tsv` records each test's result on that architecture. These
lists are the baseline the work on the system call groups starts from
(`docs/compat/COMPATIBILITY-PLAN.md`). Most tests do not pass yet, and the
notes say why, as each test reported it.

## Running them

```sh
./build-kernel.sh x86_64 dev
scripts/build-busybox-rootfs.sh all           # the base root (x86_64)
scripts/build-compat-rootfs.sh x86_64         # about five minutes the first time
scripts/run-compat-tests.sh x86_64 --jobs 3  # about an hour under KVM
```

For AArch64 and RISC-V the base root is `scripts/build-musl-rootfs.sh <arch>`,
and the run, under TCG, takes several times as long; `--jobs 7` runs seven
guests side by side, each with a share of every suite (about four hours for
all three suites on a 20-core host). Other options:

| Option | Effect |
|---|---|
| `--suites ltp-syscalls,open-posix,libc-test` | only these suites (`libc-test` alone takes minutes) |
| `--jobs N` | N guests at once |
| `--smoke` | only the tests the expected list has as `PASS` (the image's `<suite>-smoke.list`); what the gate runs, about ten minutes on x86_64 with `--jobs 3` |
| `--resume` | only the tests `results.tsv` has no result for (after an interrupted run) |
| `--first N` | start the first suite at test N |
| `--timeout N` | seconds per test |
| `--tcg` | x86_64 without KVM (refused otherwise: emulated it is about 100 times slower) |

`tools/compat/compat.py collect --arch <arch> [LOG...]` rebuilds `results.tsv`
from serial logs, for a run that stopped before writing it.

Results go to `target/compat/<arch>/`: `results.tsv` (same format as the
expected lists), `times.tsv` (milliseconds per test), `build.tsv` (tests
judged at build time) and `serial-NNN.log` (`serial-jN-NNN.log` with `--jobs`), one per boot.

The run exits non-zero when a test expected to `PASS` does not (a
regression). Tests that now pass where they did not, and other changes, are
listed but do not fail it; `--strict` fails on any difference.

## How it works

- `scripts/build-compat-rootfs.sh` builds LTP (static) and libc-test in
  `${VERIDIAN_PREFIX}/compat-build/<arch>`, copies the base root, adds the
  suites, the runner and one list per suite (`tools/compat/compat.py lists`),
  and writes `target/rootfs-compat-<arch>.img`. LTP sources that do not
  compile against musl are listed in the script with the reason, and their
  tests are `NOTBUILT`.
- `userland/tests/compat/run.sh` runs in the guest. Each test runs under
  `compat_exec` (same directory): its own process group, a time limit kept
  by polling (no `alarm`, which VeridianOS does not have yet, and which LTP's
  own timeout relies on), and its output in a file, of which a few lines
  naming the problem are printed.
- `tools/compat/compat.py run` boots QEMU, starts the runner, reads its
  `COMPAT-*` lines from the serial console, and when a test takes the guest
  down (a panic, a hang past the limit, a killed runner) records it as
  `CRASH` or `HANG` and boots again from the next test.

## Statuses

| Status | Meaning |
|---|---|
| `PASS` | exited 0 |
| `FAIL` | the test reported a failure |
| `BROK` | the test could not run its check (LTP `TBROK`, Open POSIX `UNRESOLVED`) |
| `WARN` | passed with warnings (LTP `TWARN`) |
| `CONF` | skipped: the test found something missing (LTP `TCONF`, Open POSIX `UNSUPPORTED`/`UNTESTED`) |
| `TIMEOUT` | killed at the time limit (60 s under KVM, 180 s under TCG) |
| `SIGNAL` | killed by a signal, named in the note |
| `EXECFAIL` | the program could not be started |
| `NOTBUILT` | not in the image: it does not build against musl |
| `HANG` | the guest stopped responding during the test |
| `CRASH` | the guest died, or the runner was killed, during the test |
| `FLAKY` | (expected lists only) any result is accepted; the note says why |

LTP's exit status combines its result flags, and the most serious one is
taken: `TBROK` before `TFAIL`, then `TWARN`; `TCONF` alone is `CONF`.

## Updating the expected lists

After a change that makes tests pass, run the suites and accept the result:

```sh
scripts/run-compat-tests.sh x86_64 --update
```

`--update` keeps a note written by hand while the status stays the same, and
keeps `FLAKY` entries. Commit the list with the change that moved it. A test
whose result varies between runs (timing, load) is marked `FLAKY` by hand,
with the reason in the note; a failure explained by a documented limitation
cites it, as `known: docs/KNOWN-LIMITATIONS.md (...)`.
