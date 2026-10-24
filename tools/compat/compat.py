#!/usr/bin/env python3
"""Conformance suites on VeridianOS: test lists, the QEMU driver, results.

The compat root filesystem (scripts/build-compat-rootfs.sh) holds three
suites, each with a list the in-guest runner (userland/tests/compat/run.sh)
works through:

    ltp-syscalls  LTP's runtest/syscalls
    open-posix    the Open POSIX Test Suite (runnable conformance and
                  functional tests; its compile-only tests are judged at
                  build time)
    libc-test     musl libc-test (its src/api tests are compile-only too)

Subcommands:

    lists     write the lists and build.tsv (tests judged at build time)
              from the build trees; used by the build script
    run       boot the image in QEMU, run the suites, write results.tsv,
              and compare with tests/compat/expected-<arch>.tsv
    collect   rebuild results.tsv (and times.tsv) from serial logs
    compare   compare a results.tsv with an expected list (or --update it)

Statuses (one per test):

    PASS      exited 0
    FAIL      the test reported a failure
    BROK      the test could not run its check (LTP TBROK, Open POSIX
              UNRESOLVED)
    WARN      passed with warnings (LTP TWARN)
    CONF      skipped: the test found a feature missing (LTP TCONF, Open
              POSIX UNSUPPORTED/UNTESTED)
    TIMEOUT   killed at the time limit
    SIGNAL    killed by a signal (the note names it)
    EXECFAIL  the program could not be started
    NOTBUILT  not in the image (does not build against musl)
    HANG      the guest stopped responding during the test
    CRASH     the guest died (panic, reset) or the runner was killed
    FLAKY     (expected lists only) any result is accepted

A test expected to PASS that does not is a regression, and makes `run`
and `compare` fail; other differences are reported.
"""

import argparse
import os
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
from collections import namedtuple
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SUITES = ("ltp-syscalls", "open-posix", "libc-test")
STATUSES = (
    "PASS",
    "FAIL",
    "BROK",
    "WARN",
    "CONF",
    "TIMEOUT",
    "SIGNAL",
    "EXECFAIL",
    "NOTBUILT",
    "HANG",
    "CRASH",
    "FLAKY",
)
OPEN_POSIX_ROOT = "/opt/ltp/testcases/open_posix_testsuite"
LIBC_TEST_ROOT = "/opt/libc-test"
PROMPT = "root@veridian"
PANIC = re.compile(r"KERNEL PANIC|panicked at")

_RESULT = re.compile(r"rc=(-?\d+) sig=(\d+) timeout=([01]) ms=(\d+)")
_FAILURE = re.compile(r"TFAIL|TBROK|FAIL|UNRESOLVED|[Ee]rror|failed")

# LTP result flags (include/tst_res_flags.h).
_TFAIL, _TBROK, _TWARN, _TCONF = 1, 2, 4, 32


def classify(suite, result):
    """Map the runner's RESULT for one test to (status, ms, detail)."""
    if result == "notbuilt":
        return ("NOTBUILT", 0, "")
    m = _RESULT.fullmatch(result.strip())
    if not m:
        return ("CRASH", 0, "no result from the runner")
    rc, sig, timed_out, ms = int(m[1]), int(m[2]), m[3] == "1", int(m[4])
    if timed_out:
        return ("TIMEOUT", ms, "")
    if rc < 0:
        try:
            name = signal.Signals(sig).name
        except ValueError:
            name = f"signal {sig}"
        return ("SIGNAL", ms, name)
    if rc == 0:
        return ("PASS", ms, "")
    if rc == 127:
        return ("EXECFAIL", ms, "")
    if suite == "ltp-syscalls":
        if rc & _TBROK:
            status = "BROK"
        elif rc & _TFAIL:
            status = "FAIL"
        elif rc & _TCONF:
            # LTP sets TCONF only when nothing passed (do_exit in
            # lib/tst_test.c), so a skip that also warned is still a skip.
            status = "CONF"
        elif rc & _TWARN:
            status = "WARN"
        else:
            status = "FAIL"
    elif suite == "open-posix":
        status = {1: "FAIL", 2: "BROK", 4: "CONF", 5: "CONF"}.get(rc, "FAIL")
    else:
        status = "FAIL"
    return (status, ms, "" if status != "FAIL" or rc == 1 else f"exit {rc}")


Record = namedtuple("Record", "n id result logs")


class SerialParser:
    """Follows the runner's lines in the serial output, fed in pieces."""

    def __init__(self):
        self.done = []
        self.current = None
        self.summary = None
        self.retry = None  # (n, id, reason): a test compat_exec could not start
        self.error = None
        self.panic = False
        self._buf = ""
        self._logs = {}
        self._prompt_lines = 0

    @staticmethod
    def _quoted(line):
        # A test's own output, copied by compat_exec: whatever it says, it is
        # not the shell or the kernel.
        return "COMPAT-LOG" in line

    @property
    def prompts(self):
        """Shell prompts seen; the one ending the output need not end a line."""
        partial = PROMPT in self._buf and not self._quoted(self._buf)
        return self._prompt_lines + int(partial)

    def feed(self, text):
        self._buf += text
        *lines, self._buf = self._buf.split("\n")
        for line in lines:
            line = line.rstrip("\r")
            if not self._quoted(line):
                self._prompt_lines += PROMPT in line
                self.panic = self.panic or bool(PANIC.search(line))
            self._line(line)

    def _line(self, line):
        i = line.find("COMPAT-")
        if i < 0:
            return
        word, _, rest = line[i:].partition(" ")
        if word == "COMPAT-BEGIN":
            n, _, tid = rest.partition(" ")
            if n.isdigit():
                self.current = (int(n), tid.strip())
                self._logs[int(n)] = []
        elif word == "COMPAT-LOG":
            n, _, text = rest.partition(" ")
            if n.isdigit():
                self._logs.setdefault(int(n), []).append(text)
        elif word == "COMPAT-END":
            parts = rest.split(" ", 2)
            if len(parts) == 3 and parts[0].isdigit():
                n = int(parts[0])
                self.done.append(
                    Record(n, parts[1], parts[2].strip(), self._logs.pop(n, []))
                )
                self.current = None
        elif word == "COMPAT-SUMMARY":
            n = rest.split()[-1] if rest.split() else ""
            if n.isdigit():
                self.summary = int(n)
                self.current = None
        elif word == "COMPAT-RETRY":
            parts = rest.split(" ", 2)
            if len(parts) >= 2 and parts[0].isdigit():
                self.retry = (
                    int(parts[0]),
                    parts[1],
                    parts[2].strip() if len(parts) > 2 else "",
                )
                self._logs.pop(int(parts[0]), None)
                self.current = None
        elif word == "COMPAT-ERROR":
            self.error = rest


_LTP_TMP = re.compile(r"/tmp/LTP_\w+")
_BUILD_TREE = re.compile(r"\S*/ltp-full-\d+/")
# Terminal escape sequences and other control characters (a test whose
# environment lacks LTP_COLORIZE_OUTPUT=0 colours its output), which tools
# reading the expected lists refuse.
_ESCAPE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[@-_]")
_CONTROL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f]")

# The line that explains each status best, when a test printed several.
_STATUS_TAG = {"BROK": "TBROK", "FAIL": "TFAIL", "CONF": "TCONF", "WARN": "TWARN"}


def note_for(record, detail="", status=None):
    """A one-line reason for a result that is not PASS."""
    lines = [
        _CONTROL.sub("", _ESCAPE.sub("", line)).replace("\t", " ").strip()
        for line in record.logs
    ]
    lines = [line for line in lines if line]
    tag = _STATUS_TAG.get(status)
    hit = next((line for line in lines if tag and tag in line), None)
    if hit is None:
        hit = next(
            (line for line in lines if _FAILURE.search(line)), lines[0] if lines else ""
        )
    note = "; ".join(x for x in (detail, hit) if x)
    # LTP's temporary directories have random names, and old-API tests
    # print their source's path in the build tree; keep notes stable.
    note = _BUILD_TREE.sub("", note)
    return _LTP_TMP.sub("/tmp/LTP_XXXXXX", note)[:160]


def read_tsv(path):
    rows = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            cols = line.split("\t")
            while len(cols) < 4:
                cols.append("")
            suite, tid, status, note = cols[:4]
            rows[(suite, tid)] = (status, note)
    return rows


def _order(key):
    suite, tid = key
    rank = SUITES.index(suite) if suite in SUITES else len(SUITES)
    return (
        rank,
        suite,
        [int(t) if t.isdigit() else t for t in re.split(r"(\d+)", tid)],
    )


_STATUSES_LINE = "# Statuses and the update procedure: tests/compat/README.md.\n"


def read_header(path):
    """The leading comment lines of a list, minus the standard last one, so
    that a line written by hand (a baseline's date) survives an --update."""
    lines = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            if not line.startswith("#"):
                break
            if line != _STATUSES_LINE:
                lines.append(line)
    return "".join(lines) or None


def write_tsv(path, rows, arch, header=None):
    with open(path, "w", encoding="utf-8") as f:
        f.write(
            header or f"# Conformance results for {arch}: suite, test, status, note.\n"
        )
        f.write(_STATUSES_LINE)
        for key in sorted(rows, key=_order):
            status, note = rows[key]
            f.write("\t".join((key[0], key[1], status, note.replace("\t", " "))) + "\n")


def compare(expected, actual):
    out = {"regressed": [], "improved": [], "changed": [], "missing": [], "new": []}
    for key in sorted(set(expected) | set(actual), key=_order):
        if key not in actual:
            out["missing"].append(key)
            continue
        status = actual[key][0]
        if key not in expected:
            out["new"].append(key + (status,))
            continue
        want = expected[key][0]
        if want == "FLAKY" or want == status:
            continue
        if want == "PASS":
            out["regressed"].append(key + (want, status))
        elif status == "PASS":
            out["improved"].append(key + (want, status))
        else:
            out["changed"].append(key + (want, status))
    return out


def failures(diff, expected, strict, subset):
    """What fails a run: regressions, and in a full run an expected PASS that
    did not run at all (a test that stopped being run is not a pass)."""
    bad = [x[:2] for x in diff["regressed"]]
    if not subset:
        bad += [k for k in diff["missing"] if expected[k][0] == "PASS"]
    if strict:
        bad += [x[:2] for k in ("improved", "changed", "new") for x in diff[k]]
        bad += [k for k in diff["missing"] if expected[k][0] != "PASS"]
    return bad


def uncovered(results, listed):
    """Listed tests of the suites in `results` that have no result there.

    `listed` maps a suite to the ids the image's list holds; a suite with
    no list counts as uncovered (`(no list)`), since what it should hold is
    unknown. An expected list is only rewritten from results without gaps,
    or a partial run (--first, an interrupted run's logs) would drop the
    rest of a suite.
    """
    gaps = []
    for suite in sorted({s for s, _ in results}):
        if suite not in listed:
            gaps.append((suite, "(no list)"))
            continue
        gaps += [(suite, t) for t in listed[suite] if (suite, t) not in results]
    return sorted(gaps, key=_order)


def listed_tests(root, suites):
    """The ids in the image's list of each suite (`root` is its tree)."""
    out = {}
    for suite in suites:
        lst = Path(root) / "usr" / "lib" / "compat" / f"{suite}.list"
        if lst.exists():
            out[suite] = {
                w[1]
                for w in (line.split(" ", 2) for line in lst.read_text().splitlines())
                if len(w) > 1
            }
    return out


def merge_expected(expected, actual):
    """The expected list after accepting `actual` for the suites it covers."""
    suites = {suite for suite, _ in actual}
    merged = {k: v for k, v in expected.items() if k[0] not in suites}
    for key, (status, note) in actual.items():
        old = expected.get(key)
        if old and (old[0] == "FLAKY" or old[0] == status):
            merged[key] = old
        else:
            merged[key] = (status, note)
    return merged


# --- lists -----------------------------------------------------------------


def runtest_entries(text):
    entries = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        # Fields are separated by spaces or tabs (runtest files use both).
        words = line.split()
        entries.append((words[0], " ".join(words[1:]) or words[0]))
    return entries


def open_posix_prefix(d):
    """The name prefix generate-makefiles.sh gives the tests in `d`."""
    base = os.path.basename(d)
    if base == "speculative":
        return os.path.basename(os.path.dirname(d)) + "_speculative"
    return base


def open_posix_entry(path):
    """(id, directory, command) for a runnable source from locate-test."""
    rel = path.removeprefix("./")
    d, name = os.path.split(rel)
    stem, ext = os.path.splitext(name)
    prog = f"./{open_posix_prefix(d)}_{stem}" + (".run-test" if ext == ".c" else ".sh")
    return (f"{d}/{stem}", f"{OPEN_POSIX_ROOT}/{d}", prog)


def make_reaches(top, d):
    """Whether upstream's build enters `d`: each level needs a Makefile."""
    parts = Path(d).parts
    return all(
        (Path(top, *parts[: i + 1]) / "Makefile").exists() for i in range(len(parts))
    )


_COMPILE_ERROR = re.compile(
    r"^(?:\.\./)*((?:conformance|functional|stress)/\S+?)\.c:\d+:\d+: (error: .*)$",
    re.MULTILINE,
)


_VERDICT = re.compile(r"^(\S+) (?:compile|link) (PASSED|FAILED)")
_LINK_ERROR = re.compile(r"(undefined reference to .*|error: .*)$")


def compile_errors(log):
    """First compiler or linker error per Open POSIX test, from the build log.

    A compiler error names its source; a linker error does not, so it goes
    with the next verdict line upstream prints ("ID compile FAILED").
    """
    errors = {}
    for m in _COMPILE_ERROR.finditer(log):
        errors.setdefault(m[1], m[2].strip()[:160])
    pending = None
    for line in log.splitlines():
        v = _VERDICT.match(line)
        if v:
            if v[2] == "FAILED" and pending and v[1] not in errors:
                errors[v[1]] = pending[:160]
            pending = None
            continue
        e = _LINK_ERROR.search(line)
        if e and pending is None and "ld returned" not in line:
            pending = e[1].strip()
    return errors


LTP_BIN = "/opt/ltp/testcases/bin"


def list_lines(entries, root):
    """The runner's lines for (id, dir, command) entries: "N ID DIR PATH ARGS".

    PATH is the program's path in the image (a bare name is an LTP binary,
    which the runner's PATH finds in LTP_BIN; a relative one is in DIR), or
    `-` when the image under `root` does not have it. Resolving it here
    keeps the runner to shell builtins.
    """
    lines = []
    for n, (tid, d, cmd) in enumerate(entries, 1):
        prog, _, rest = cmd.partition(" ")
        if prog.startswith("/"):
            path = prog
        elif "/" in prog:
            path = f"{'/tmp' if d == '-' else d}/{prog.removeprefix('./')}"
        else:
            path = f"{LTP_BIN}/{prog}"
        if not os.path.isfile(os.path.join(root, path.lstrip("/"))):
            path = "-"
        lines.append(" ".join(x for x in (str(n), tid, d, path, rest.strip()) if x))
    return lines


def smoke_lines(lines, suite, expected):
    """The lines of tests `expected` has as PASS, numbered again from 1."""
    out = []
    for line in lines:
        words = line.split(" ", 2)
        if len(words) >= 2 and expected.get((suite, words[1]), ("",))[0] == "PASS":
            out.append(
                f"{len(out) + 1} {words[1]} {words[2] if len(words) > 2 else ''}".rstrip()
            )
    return out


def suite_of(list_name):
    """The suite a list belongs to (`<suite>-smoke` is a subset of `<suite>`)."""
    return list_name.removesuffix("-smoke")


def only_run(expected, results):
    """The expected entries for the tests that ran (a smoke run is a subset)."""
    return {k: v for k, v in expected.items() if k in results}


def _write_list(path, entries, root):
    with open(path, "w", encoding="utf-8") as f:
        f.writelines(line + "\n" for line in list_lines(entries, root))


def cmd_lists(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    build = {}
    log = Path(args.make_log).read_text(errors="replace") if args.make_log else ""

    # LTP syscalls: runtest/syscalls as it stands; sources left out of the
    # build (they do not compile against musl) are NOTBUILT with the reason.
    ltp = Path(args.ltp_src)
    entries = runtest_entries((ltp / "runtest" / "syscalls").read_text())
    _write_list(out / "ltp-syscalls.list", [(t, "-", c) for t, c in entries], args.root)
    removed = {}
    if args.removed:
        for line in Path(args.removed).read_text().splitlines():
            if line.strip() and not line.startswith("#"):
                src, _, reason = line.partition("\t")
                removed[Path(src).stem] = reason.strip()
    for tid, cmd in entries:
        prog = cmd.split()[0]
        if prog in removed:
            build[("ltp-syscalls", tid)] = ("NOTBUILT", f"not built: {removed[prog]}")

    # Open POSIX: upstream's locate-test decides what is a test.
    posix = ltp / "testcases" / "open_posix_testsuite"
    stage = Path(args.ltp_stage) / "testcases" / "open_posix_testsuite"
    errors = compile_errors(log)

    def locate(mode):
        r = subprocess.run(
            ["sh", "scripts/locate-test", mode, "."],
            cwd=posix,
            capture_output=True,
            text=True,
            check=True,
        )
        # Only what upstream's own build compiles (and its run-tests runs):
        # not the stress tests, nor directories its Makefiles never enter.
        return sorted(
            p
            for p in r.stdout.split()
            if "/stress/" not in p
            and make_reaches(posix, os.path.dirname(p.removeprefix("./")))
        )

    posix_entries = [open_posix_entry(p) for p in locate("--runnable")]
    _write_list(out / "open-posix.list", posix_entries, args.root)
    for tid, d, prog in posix_entries:
        rel = d[len(OPEN_POSIX_ROOT) + 1 :]
        if not (stage / rel / prog[2:]).exists():
            why = errors.get(tid, "does not compile against musl")
            build[("open-posix", tid)] = ("NOTBUILT", f"not built: {why}")
    for p in locate("--buildonly"):
        rel = p.removeprefix("./")
        d, name = os.path.split(rel)
        stem = os.path.splitext(name)[0]
        tid = f"{d}/{stem}"
        if (posix / d / f"{open_posix_prefix(d)}_{stem}.test").exists():
            build[("open-posix", tid)] = ("PASS", "compile-only")
        else:
            why = errors.get(tid, "does not compile")
            build[("open-posix", tid)] = ("NOTBUILT", f"compile-only: {why}")

    # libc-test: the binaries its Makefile builds; src/api is compile-only.
    lt = Path(args.libc_test)
    with tempfile.NamedTemporaryFile("w", suffix=".mk", delete=False) as mk:
        mk.write("print-names:\n\t@printf '%s\\n' $(NAMES)\n")
        mk.write("print-bins:\n\t@printf '%s\\n' $(foreach n,$(NAMES),$($(n).BINS))\n")
    try:

        def make(target):
            r = subprocess.run(
                ["make", "-s", "-f", "Makefile", "-f", mk.name, target],
                cwd=lt,
                capture_output=True,
                text=True,
                check=True,
            )
            return r.stdout.split()

        bins = [b for b in make("print-bins") if b.endswith(".exe")]
        names = make("print-names")
    finally:
        os.unlink(mk.name)
    lt_log = (
        Path(args.libc_test_log).read_text(errors="replace")
        if args.libc_test_log
        else ""
    )
    lt_entries = []
    for b in bins:
        tid = b[len("src/") : -len(".exe")]
        lt_entries.append((tid, LIBC_TEST_ROOT, b))
        if not (lt / b).exists():
            build[("libc-test", tid)] = (
                "NOTBUILT",
                "not built (see the libc-test build log)",
            )
    _write_list(out / "libc-test.list", lt_entries, args.root)
    for name in names:
        if not name.startswith("api/"):
            continue
        if (lt / "src" / f"{name}.o").exists():
            build[("libc-test", name)] = ("PASS", "compile-only")
        else:
            m = re.search(
                rf"^src/{re.escape(name)}\.c:\d+:\d+: (error: .*)$",
                lt_log,
                re.MULTILINE,
            )
            why = m[1][:150] if m else "does not compile"
            build[("libc-test", name)] = ("NOTBUILT", f"compile-only: {why}")

    # The smoke lists: the tests the expected list has as passing, for a
    # quick regression check (scripts/dev/gate.sh --rootfs).
    if args.expected and Path(args.expected).exists():
        expected = read_tsv(args.expected)
        for name in SUITES:
            lines = (out / f"{name}.list").read_text().splitlines()
            smoke = smoke_lines(lines, name, expected)
            (out / f"{name}-smoke.list").write_text("".join(f"{x}\n" for x in smoke))
            print(f"{name}-smoke: {len(smoke)} tests")

    write_tsv(
        args.build_tsv,
        build,
        args.arch,
        header=f"# Tests judged at build time for {args.arch}: suite, test, status, note.\n",
    )
    for name in SUITES:
        count = len((out / f"{name}.list").read_text().splitlines())
        print(f"{name}: {count} tests")
    print(f"build-time results: {len(build)}")


# --- run -------------------------------------------------------------------


def qemu_command(arch, image, smp, tcg=False):
    if arch == "x86_64":
        ovmf = next(
            (
                f
                for f in (
                    "/usr/share/edk2/x64/OVMF.4m.fd",
                    "/usr/share/OVMF/OVMF_CODE_4M.fd",
                    "/usr/share/OVMF/OVMF_CODE.fd",
                    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
                    "/usr/share/qemu/OVMF.fd",
                )
                if os.access(f, os.R_OK)
            ),
            None,
        )
        if not ovmf:
            sys.exit("FAIL: no OVMF firmware found")
        # x86_64 is run under KVM: emulated it is about 100 times slower, past
        # what the limits allow for (--tcg runs it anyway, with TCG limits).
        if tcg:
            accel = ["-cpu", "qemu64"]
        elif os.access("/dev/kvm", os.W_OK):
            accel = ["-enable-kvm", "-cpu", "host"]
        else:
            sys.exit(
                "FAIL: /dev/kvm is not writable; x86_64 runs under KVM (--tcg to emulate)"
            )
        kernel = REPO / "target/x86_64-veridian/debug/veridian-uefi.img"
        cmd = [
            "qemu-system-x86_64",
            *accel,
            "-drive",
            f"if=pflash,format=raw,readonly=on,file={ovmf}",
            "-drive",
            f"id=disk0,if=none,format=raw,file={kernel},snapshot=on",
            "-device",
            "ide-hd,drive=disk0",
            "-drive",
            f"file={image},if=none,id=vd0,format=raw,snapshot=on",
            "-device",
            "virtio-blk-pci,drive=vd0",
        ]
    elif arch in ("aarch64", "riscv64"):
        if arch == "aarch64":
            kernel = (
                REPO / "target/aarch64-unknown-none-softfloat/debug/veridian-kernel"
            )
            cmd = ["qemu-system-aarch64", "-M", "virt", "-cpu", "cortex-a72"]
        else:
            kernel = REPO / "target/riscv64imac-unknown-none-elf/debug/veridian-kernel"
            cmd = ["qemu-system-riscv64", "-M", "virt", "-bios", "default"]
        cmd += [
            "-kernel",
            str(kernel),
            "-device",
            "ramfb",
            "-drive",
            f"file={image},if=none,id=vd0,format=raw,snapshot=on",
            "-device",
            "virtio-blk-device,drive=vd0",
        ]
    else:
        sys.exit(f"FAIL: unknown arch {arch}")
    for f in (kernel, image):
        if not os.access(f, os.R_OK):
            sys.exit(f"FAIL: {f} not built")
    return cmd + [
        "-smp",
        str(smp),
        "-serial",
        "stdio",
        "-display",
        "none",
        "-m",
        "2048M",
        "-no-reboot",
    ]


class Guest:
    """One QEMU boot, its serial output in a log file."""

    def __init__(self, cmd, log_path):
        self.log_path = log_path
        # Both handles live as long as the QEMU process; stop() closes them.
        self._log = open(log_path, "wb")  # noqa: SIM115
        self.proc = subprocess.Popen(
            cmd, stdin=subprocess.PIPE, stdout=self._log, stderr=subprocess.STDOUT
        )
        self._read = open(log_path, "rb")  # noqa: SIM115

    def read(self):
        return self._read.read().decode("utf-8", errors="replace")

    def send(self, line):
        self.proc.stdin.write(line.encode() + b"\r")
        self.proc.stdin.flush()

    def alive(self):
        return self.proc.poll() is None

    def stop(self):
        if self.alive():
            self.proc.kill()
        self.proc.wait()
        for f in (self.proc.stdin, self._log, self._read):
            try:
                f.close()
            except OSError:
                pass


def _boot(cmd, log_path, parser, wait):
    guest = Guest(cmd, log_path)
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline and guest.alive():
        parser.feed(guest.read())
        if parser.prompts:
            time.sleep(2)
            parser.feed(guest.read())
            return guest
        time.sleep(0.5)
    guest.stop()
    return None


def stalled(now, since, heard, limit, grace):
    """Whether a guest with no result since `since` has stopped responding.

    Past the test's limit plus `grace` it has, unless it is still writing to
    the console (killing a test's thousand processes takes a while, each
    exit logged); a guest still talking gets four times the grace.
    """
    if now - since <= limit + grace:
        return False
    return now - heard > 30 or now - since > limit + 4 * grace


class RunError(Exception):
    pass


def split_ranges(count, jobs):
    """`count` tests in `jobs` contiguous (first, last) ranges, none empty."""
    jobs = max(1, min(jobs, count))
    size, extra = divmod(count, jobs)
    ranges, first = [], 1
    for j in range(jobs):
        last = first + size - 1 + (1 if j < extra else 0)
        ranges.append((first, last))
        first = last + 1
    return ranges


def missing_ranges(lines, suite, have):
    """Contiguous (first, last) runs of the list's tests not in `have`."""
    ranges = []
    for line in lines:
        words = line.split(" ", 2)
        if len(words) < 2 or not words[0].isdigit():
            continue
        n = int(words[0])
        if (suite, words[1]) in have:
            continue
        if ranges and ranges[-1][1] == n - 1:
            ranges[-1] = (ranges[-1][0], n)
        else:
            ranges.append((n, n))
    return ranges


def share_ranges(ranges, jobs):
    """(list, first, last) ranges cut into `jobs` shares of about equal size."""
    total = sum(last - first + 1 for _, first, last in ranges)
    if total == 0:
        return []
    target = -(-total // max(1, min(jobs, total)))
    shares, share, size = [], [], 0
    for name, first, last in ranges:
        while first <= last:
            take = min(last - first + 1, target - size)
            share.append((name, first, first + take - 1))
            size += take
            first += take
            if size == target:
                shares.append(share)
                share, size = [], 0
    if share:
        shares.append(share)
    return shares


def stale_smoke(expected, lists, exclude=()):
    """Expected passes missing from the image's smoke lists (`<suite>-smoke`).

    The lists are made from the expected list when the image is built; one
    built before a test was added as PASS would leave it out of a smoke run
    without a word. `exclude` holds tests judged at build time, which have
    no list entry.
    """
    stale = []
    for name, lines in lists.items():
        suite = suite_of(name)
        ids = {w[1] for w in (line.split(" ", 2) for line in lines) if len(w) > 1}
        stale += [
            k
            for k, (st, _) in expected.items()
            if k[0] == suite and st == "PASS" and k[1] not in ids and k not in exclude
        ]
    return sorted(stale, key=_order)


def retry_ranges(keys, lists):
    """Single-test ranges, (list, n, n), for (suite, id) keys in these lists.

    `lists` maps a list's name (`<suite>` or `<suite>-smoke`) to its lines;
    a key whose test is in none of them is left out.
    """
    where = {}
    for name, lines in lists.items():
        for line in lines:
            words = line.split(" ", 2)
            if len(words) >= 2 and words[0].isdigit():
                where[(suite_of(name), words[1])] = (name, int(words[0]))
    return sorted({where[k] + (where[k][1],) for k in keys if k in where})


def run_retries(run, arch, args, lists):
    """Run each test that regressed once more; it counts only if it fails again.

    A test whose result varies (timing, load) would otherwise fail a gate
    that then has to run again in full. One that passes the second time is
    reported as a FLAKY candidate rather than a regression.
    """
    expected_path = Path(
        args.expected or REPO / "tests" / "compat" / f"expected-{arch}.tsv"
    )
    if not expected_path.exists():
        return []
    expected = read_tsv(expected_path)
    if args.smoke or args.first:
        expected = only_run(expected, run.results)
    else:
        expected = {
            k: v for k, v in expected.items() if k[0] in {s for s, _ in run.results}
        }
    regressed = [x[:2] for x in compare(expected, run.results)["regressed"]]
    ranges = retry_ranges(regressed, lists)
    if not ranges:
        return []
    first = {k: run.results[k] for k in regressed}
    print(f"retrying {len(ranges)} tests that regressed", flush=True)
    # Each on a fresh guest: what the first pass left behind (memory, files,
    # a wedged device) is what makes a result vary in the first place.
    Worker(run, "retry", ranges, fresh=True)()
    if run.error:
        # The first pass finished: keep its results (and the retries' so far).
        name = "smoke-results.tsv" if args.smoke else "results.tsv"
        write_tsv(run.out / name, run.results, arch)
        sys.exit(f"FAIL: retry: {run.error} (results in {run.out / name})")
    flaky = []
    for key in regressed:
        status, _ = run.results.get(key, ("CRASH", ""))
        if status == expected[key][0]:
            flaky.append(key)
            run.results[key] = (
                status,
                f"{first[key][0]} once, then {status} on a retry",
            )
    return flaky


class Worker:
    """One guest working through (suite, first, last) ranges in turn.

    When a test takes the guest down, the test is recorded (CRASH or HANG)
    and a new guest carries on after it.
    """

    def __init__(self, run, name, ranges, fresh=False):
        self.run = run
        self.name = name
        self.ranges = ranges
        self.fresh = fresh  # a new guest for every range
        self.guest = None
        self.boot = 0
        self.retried = None  # the last test run again after it could not start

    def _boot(self):
        run = self.run
        with run.lock:
            run.boots += 1
            if run.boots > run.max_boots:
                raise RunError(f"more than {run.max_boots} boots; giving up")
        # The next free name: a resumed run keeps the earlier logs.
        suffix = run.log_prefix + (f"{self.name}-" if self.name else "")
        while True:
            self.boot += 1
            log = run.out / f"serial-{suffix}{self.boot:03d}.log"
            if not log.exists():
                break
        self.used = False  # no range finished on this guest yet
        self.guest = _boot(run.cmd, log, SerialParser(), run.boot_wait)
        if self.guest is None:
            raise RunError(f"no shell prompt within {run.boot_wait}s (log: {log})")

    def _record(self, suite, parser):
        run = self.run
        with run.lock:
            for r in parser.done:
                status, ms, detail = classify(suite, r.result)
                note = "" if status == "PASS" else note_for(r, detail, status)
                key = (suite, r.id)
                if status == "NOTBUILT" and key in run.results:
                    note = run.results[key][1] or note
                run.results[key] = (status, note)
                run.times[key] = ms

    def _range(self, list_name, first, last):
        run = self.run
        suite = suite_of(list_name)
        tag = f"[{list_name}{' ' + self.name if self.name else ''}]"
        while True:
            if run.stop.is_set():
                return
            if self.guest is not None and not self.guest.alive():
                # It died after the last range finished: a new one goes on.
                self.guest.stop()
                self.guest = None
            if self.guest is None:
                self._boot()
            parser = SerialParser()
            self.guest.send(
                f"/bin/ash /usr/lib/compat/run.sh /usr/lib/compat/{list_name}.list "
                f"{first} {run.limit} {last}"
            )
            print(f"{tag} tests {first}-{last} (boot {self.boot})", flush=True)
            seen, since = 0, time.monotonic()
            heard = since  # the last serial output of any kind
            outcome = None
            while outcome is None:
                time.sleep(0.5)
                if run.stop.is_set():
                    return
                text = self.guest.read()
                if text:
                    parser.feed(text)
                    heard = time.monotonic()
                if len(parser.done) != seen:
                    since = time.monotonic()
                    if len(parser.done) // 50 != seen // 50:
                        r = parser.done[-1]
                        print(f"{tag} {r.n} {r.id}", flush=True)
                    seen = len(parser.done)
                if parser.summary is not None:
                    outcome = "done"
                elif parser.retry:
                    # The runner stops after a test that could not start;
                    # there is nothing more to wait for on this guest.
                    outcome = "RETRY"
                elif parser.error:
                    raise RunError(f"runner: {parser.error}")
                elif parser.panic or not self.guest.alive():
                    outcome = "CRASH"
                elif parser.prompts > 0:
                    # Back at the prompt without the summary line: the
                    # runner was killed.
                    time.sleep(1)
                    parser.feed(self.guest.read())
                    outcome = "done" if parser.summary is not None else "CRASH"
                elif stalled(time.monotonic(), since, heard, run.limit, run.grace):
                    outcome = "HANG"
            self._record(suite, parser)
            if outcome == "done":
                # A summary alone does not show that the range ran: every
                # test in it needs a result (a runner that stopped reading
                # its list early still prints one).
                got = {r.n for r in parser.done}
                gaps = [n for n in range(first, last + 1) if n not in got]
                if gaps or parser.summary != last:
                    raise RunError(
                        f"{list_name} tests {first}-{last}: the runner finished with "
                        f"{len(gaps)} without a result (first {gaps[:5]}), summary "
                        f"{parser.summary} (log {self.guest.log_path})"
                    )
                self.used = True
                if self.fresh:
                    # The next range boots another guest: no prompt to wait for.
                    self.guest.stop()
                    self.guest = None
                    return
                # The shell prints its prompt after the summary line; read it
                # here, or the next range's parser takes it for the runner
                # having died.
                deadline = time.monotonic() + run.grace
                while (
                    parser.prompts == 0
                    and time.monotonic() < deadline
                    and self.guest.alive()
                ):
                    time.sleep(0.5)
                    parser.feed(self.guest.read())
                if parser.prompts == 0:
                    # No prompt after the summary: the next range would wait a
                    # whole limit on this guest before booting another.
                    self.guest.stop()
                    self.guest = None
                return
            if parser.retry:
                n, tid, why = parser.retry
                if self.retried == (suite, n):
                    # Not on a fresh guest either: record it and go on.
                    with run.lock:
                        run.results[(suite, tid)] = (
                            "EXECFAIL",
                            f"could not be started: {why}",
                        )
                    print(f"{tag} EXECFAIL at {n} {tid}: {why} (twice)", flush=True)
                    first = n + 1
                else:
                    print(
                        f"{tag} {n} {tid} could not be started ({why}); booting again",
                        flush=True,
                    )
                    self.retried = (suite, n)
                    first = n
            elif parser.current:
                n, tid = parser.current
                if parser.panic:
                    why = "kernel panic"
                elif not self.guest.alive():
                    why = "guest exited"
                elif outcome == "CRASH":
                    why = "runner killed"
                else:
                    why = "no output"
                with run.lock:
                    run.results[(suite, tid)] = (outcome, why)
                print(f"{tag} {outcome} at {n} {tid}: {why}", flush=True)
                first = n + 1
            elif parser.done:
                # Stuck between two tests: carry on after the last one.
                first = parser.done[-1].n + 1
            elif self.used:
                # Earlier ranges ran on this guest and now nothing starts:
                # something they did left the guest stuck. A new one goes on.
                print(
                    f"{tag} the guest stopped before test {first} ({outcome}); booting again",
                    flush=True,
                )
                with run.lock:
                    run.stuck.append(
                        f"{self.guest.log_path}: before {suite} test {first}"
                    )
            else:
                # A fresh guest started no test: booting again would only do
                # the same.
                log = self.guest.log_path
                self.guest.stop()
                tail = "\n".join(
                    Path(log).read_text(errors="replace").splitlines()[-15:]
                )
                raise RunError(
                    f"the runner started no test ({outcome}); log {log}:\n{tail}"
                )
            self.guest.stop()
            self.guest = None
            if first > last:
                return

    def __call__(self):
        try:
            for suite, first, last in self.ranges:
                if self.fresh and self.guest is not None:
                    self.guest.stop()
                    self.guest = None
                self._range(suite, first, last)
        except RunError as e:
            self.run.error = self.run.error or str(e)
            self.run.stop.set()
        except Exception as e:  # report it, and stop the other workers
            self.run.error = self.run.error or f"{type(e).__name__}: {e}"
            self.run.stop.set()
            raise
        finally:
            if self.guest is not None:
                self.guest.stop()


def cmd_run(args):
    arch = args.arch
    tcg = arch != "x86_64" or args.tcg
    run = argparse.Namespace()
    run.out = Path(args.out or REPO / "target" / "compat" / arch)
    run.out.mkdir(parents=True, exist_ok=True)
    image = Path(args.image or REPO / "target" / f"rootfs-compat-{arch}.img")
    root = Path(args.root or REPO / "target" / f"rootfs-compat-{arch}")
    run.limit = args.timeout or (180 if tcg else 60)
    run.boot_wait = args.boot_wait or (900 if tcg else 300)
    run.grace = args.grace or (300 if tcg else 90)
    run.cmd = qemu_command(arch, image, args.smp, tcg=args.tcg)
    run.max_boots = args.max_boots
    run.boots = 0
    run.lock = threading.Lock()
    run.stop = threading.Event()
    run.error = None
    run.stuck = []
    run.log_prefix = "smoke-" if args.smoke else ""
    run.results, run.times = {}, {}
    suites = args.suites.split(",") if args.suites else list(SUITES)
    if args.jobs > 1 and args.first:
        sys.exit("FAIL: --first and --jobs do not go together")
    if args.update and (args.smoke or args.first):
        sys.exit("FAIL: --update needs whole suites, not --smoke or --first")
    # The tests judged at build time, of the suites being run (a smoke run
    # compares them too: a compile-only test has no list entry).
    build_tsv = Path(args.build_tsv or run.out / "build.tsv")
    if not build_tsv.exists():
        # The image carries a copy, should the output directory be new.
        build_tsv = root / "usr" / "lib" / "compat" / "build.tsv"
    if build_tsv.exists():
        run.results.update(
            {k: v for k, v in read_tsv(build_tsv).items() if k[0] in suites}
        )
    listed = listed_tests(root, suites)
    if args.smoke:
        suites = [f"{s}-smoke" for s in suites]
        expected_path = Path(
            args.expected or REPO / "tests" / "compat" / f"expected-{arch}.tsv"
        )
        if expected_path.exists():
            smoke = {}
            for name in suites:
                lst = root / "usr" / "lib" / "compat" / f"{name}.list"
                smoke[name] = lst.read_text().splitlines() if lst.exists() else []
            stale = stale_smoke(
                read_tsv(expected_path), smoke, exclude=set(run.results)
            )
            if stale:
                sys.exit(
                    f"FAIL: the image's smoke lists lack {len(stale)} expected passes "
                    f"(first {' '.join(stale[0])}); rebuild it: scripts/build-compat-rootfs.sh {arch}"
                )

    have = set()
    if args.resume:
        if args.smoke or args.first:
            sys.exit("FAIL: --resume goes with neither --smoke nor --first")
        previous = run.out / "results.tsv"
        if previous.exists():
            kept = {
                k: v
                for k, v in read_tsv(previous).items()
                if v[0] != "NOTBUILT" and k[0] in {suite_of(x) for x in suites}
            }
            run.results.update(kept)
            have = set(kept)
        # And the run times measured then, which times.tsv is rewritten with.
        previous_times = run.out / "times.tsv"
        if previous_times.exists():
            for line in previous_times.read_text().splitlines():
                if not line.strip() or line.startswith("#"):
                    continue
                suite, tid, ms = line.split("\t")
                if (suite, tid) in have:
                    run.times[(suite, tid)] = int(ms)

    # Each worker takes its share of every suite, so they finish together.
    shares = [[] for _ in range(args.jobs)]
    pending = []
    for si, suite in enumerate(suites):
        lst = root / "usr" / "lib" / "compat" / f"{suite}.list"
        if not lst.exists():
            sys.exit(f"FAIL: no {lst} (run scripts/build-compat-rootfs.sh {arch})")
        lines = lst.read_text().splitlines()
        if not lines:
            continue
        if args.resume:
            pending += [
                (suite, f, b) for f, b in missing_ranges(lines, suite_of(suite), have)
            ]
            continue
        if args.jobs == 1:
            # --first is where the first suite starts; the others start at 1.
            shares[0].append((suite, (args.first or 1) if si == 0 else 1, len(lines)))
            continue
        for j, (first, last) in enumerate(split_ranges(len(lines), args.jobs)):
            shares[j].append((suite, first, last))
    if args.resume:
        shares = share_ranges(pending, args.jobs)
        print(
            f"resuming: {sum(b - a + 1 for _, a, b in pending)} tests without a result",
            flush=True,
        )
    if not args.resume:
        # A new run's logs only (once every list has been found): `collect`
        # reads every serial-*.log here, and an earlier run's would be taken
        # as this one's.
        # A smoke run keeps its own logs, apart from a full run's.
        mine = "serial-smoke-*.log" if args.smoke else "serial-*.log"
        for old in run.out.glob(mine):
            if args.smoke or not old.name.startswith("serial-smoke-"):
                old.unlink()
    names = [""] if args.jobs == 1 else [f"j{j + 1}" for j in range(args.jobs)]
    workers = [Worker(run, n, share) for n, share in zip(names, shares) if share]
    threads = [threading.Thread(target=w, name=w.name or "worker") for w in workers]
    try:
        for t in threads:
            t.start()
        for t in threads:
            while t.is_alive():
                t.join(1)
    except KeyboardInterrupt:
        run.stop.set()
        for t in threads:
            t.join()
        raise
    if run.error:
        # Keep what finished (`collect` can add the rest from the logs), and
        # in a smoke run not over a full run's results.
        name = "smoke-results.tsv" if args.smoke else "results.tsv"
        write_tsv(run.out / name, run.results, arch)
        sys.exit(f"FAIL: {run.error} (partial results in {run.out / name})")

    flaky = []
    if args.retry and not args.update:
        lists = {
            name: (root / "usr" / "lib" / "compat" / f"{name}.list")
            .read_text()
            .splitlines()
            for name in suites
        }
        flaky = run_retries(run, arch, args, lists)
        for key in flaky:
            print(
                f"FLAKY candidate (failed once, passed on a retry): {key[0]} {key[1]}"
            )

    if args.smoke:
        write_tsv(run.out / "smoke-results.tsv", run.results, arch)
        print(f"boots: {run.boots}")
        return report(
            arch,
            run.results,
            Path(args.expected) if args.expected else None,
            False,
            args.strict,
            subset=True,
        )
    write_tsv(run.out / "results.tsv", run.results, arch)
    with open(run.out / "times.tsv", "w", encoding="utf-8") as f:
        f.writelines(
            f"{key[0]}\t{key[1]}\t{run.times[key]}\n"
            for key in sorted(run.times, key=_order)
        )
    print(f"boots: {run.boots}")
    for line in run.stuck:
        print(f"guest stuck between tests: {line}")
    return report(
        arch,
        run.results,
        Path(args.expected) if args.expected else None,
        args.update,
        args.strict,
        # Tests --first skipped did not run; they are not missing.
        subset=bool(args.first),
        listed=listed,
    )


def _summary(results):
    lines = []
    for suite in SUITES:
        counts = {}
        for (s, _), (status, _) in results.items():
            if s == suite:
                counts[status] = counts.get(status, 0) + 1
        if counts:
            total = sum(counts.values())
            parts = " ".join(f"{k}={counts[k]}" for k in STATUSES if k in counts)
            lines.append(
                f"COMPAT {suite}: {counts.get('PASS', 0)}/{total} pass ({parts})"
            )
    return lines


def report(
    arch, results, expected_path, update, strict, subset=False, listed=None, suites=()
):
    for line in _summary(results):
        print(line)
    expected_path = expected_path or REPO / "tests" / "compat" / f"expected-{arch}.tsv"
    if update:
        gaps = uncovered(results, listed or {})
        if gaps:
            print(
                f"FAIL: not updating {expected_path}: {len(gaps)} listed tests have no result"
            )
            for key in gaps[:20]:
                print("  " + " ".join(key))
            return 2
        expected, header = {}, None
        if expected_path.exists():
            expected, header = read_tsv(expected_path), read_header(expected_path)
        write_tsv(expected_path, merge_expected(expected, results), arch, header)
        print(f"updated {expected_path}")
        return 0
    if not expected_path.exists():
        print(
            f"no expected list at {expected_path}; nothing to compare (--update writes one)"
        )
        return 0
    # The suites compared: those with results, and any the caller knows were
    # started (a suite that stopped before its first test has none).
    suites = set(suites) | {s for s, _ in results}
    expected = {k: v for k, v in read_tsv(expected_path).items() if k[0] in suites}
    if subset:
        expected = only_run(expected, results)
    diff = compare(expected, results)
    for kind in ("regressed", "improved", "changed", "new", "missing"):
        items = diff[kind]
        if not items:
            continue
        print(f"{kind}: {len(items)}")
        for item in items[:40]:
            print("  " + " ".join(item))
        if len(items) > 40:
            print(f"  ... {len(items) - 40} more")
    bad = failures(diff, expected, strict, subset)
    absent = [k for k in diff["missing"] if expected[k][0] == "PASS"]
    if absent and not subset:
        print(f"expected to pass but not run: {len(absent)}")
    print(
        ("FAIL" if bad else "PASS")
        + f" compat {arch}: {len(diff['regressed'])} regressed, "
        f"{len(diff['improved'])} improved, {len(diff['changed'])} changed"
    )
    return 1 if bad else 0


_RUN_LINE = re.compile(r"run\.sh /usr/lib/compat/(\S+)\.list")


def _collect_finish(results, suite, parser, times):
    if parser is None:
        return
    for r in parser.done:
        status, ms, detail = classify(suite, r.result)
        if times is not None:
            times[(suite, r.id)] = ms
        results[(suite, r.id)] = (
            status,
            "" if status == "PASS" else note_for(r, detail, status),
        )
    if parser.current:
        status = "CRASH" if parser.panic else "HANG"
        results[(suite, parser.current[1])] = (
            status,
            "no result (from the serial log)",
        )


def results_from_logs(texts, times=None, attempted=None):
    """Results read back from serial logs (`collect`), in the order given.

    A test that began and never ended is CRASH if the kernel panicked after
    it began, HANG otherwise; the driver's own record of why is not in the
    log. `times`, if given, gets each test's run time in milliseconds;
    `attempted`, if given, every suite the logs started, with results or not.
    """
    results = {}
    for text in texts:
        suite, parser = None, None
        for line in text.splitlines():
            m = _RUN_LINE.search(line)
            if m and "COMPAT-" not in line:
                if attempted is not None:
                    attempted.add(suite_of(m[1]))
                if suite_of(m[1]) != suite or (parser and parser.current):
                    _collect_finish(results, suite, parser, times)
                    suite, parser = suite_of(m[1]), SerialParser()
                continue
            if parser is not None:
                parser.feed(line + "\n")
        _collect_finish(results, suite, parser, times)
    return results


def merge_build_rows(logged, build, attempted=()):
    """The results of the logs plus the build-time rows of the suites they ran.

    Build rows (tests judged when the image was built, such as NOTBUILT) of a
    suite the logs neither started (`attempted`) nor have a result for are
    left out, as `run` does: otherwise collecting one suite's logs would
    report every other suite as built but not run. A logged NOTBUILT keeps
    the build row's note.
    """
    suites = {k[0] for k in logged} | set(attempted)
    results = {k: v for k, v in build.items() if k[0] in suites}
    for key, value in logged.items():
        if value[0] == "NOTBUILT" and key in results:
            value = (value[0], results[key][1] or value[1])
        results[key] = value
    return results


def cmd_collect(args):
    out = Path(args.out or REPO / "target" / "compat" / args.arch)
    # A full run's logs (smoke runs keep theirs as serial-smoke-*).
    logs = sorted(
        (p for p in out.glob("serial-*.log") if not p.name.startswith("serial-smoke-")),
        key=lambda p: p.stat().st_mtime,
    )
    if args.logs:
        logs = [Path(p) for p in args.logs]
    build_tsv = out / "build.tsv"
    if not build_tsv.exists():
        build_tsv = (
            Path(args.root or REPO / "target" / f"rootfs-compat-{args.arch}")
            / "usr/lib/compat/build.tsv"
        )
    times, attempted = {}, set()
    texts = (p.read_text(errors="replace") for p in logs)
    results = merge_build_rows(
        results_from_logs(texts, times, attempted),
        read_tsv(build_tsv) if build_tsv.exists() else {},
        attempted,
    )
    write_tsv(out / "results.tsv", results, args.arch)
    with open(out / "times.tsv", "w", encoding="utf-8") as f:
        f.writelines(f"{k[0]}\t{k[1]}\t{times[k]}\n" for k in sorted(times, key=_order))
    print(
        f"collected {len(results)} results from {len(logs)} logs into {out / 'results.tsv'}"
    )
    return report(
        args.arch,
        results,
        Path(args.expected) if args.expected else None,
        args.update,
        args.strict,
        listed=listed_tests(
            args.root or REPO / "target" / f"rootfs-compat-{args.arch}", SUITES
        ),
        suites=attempted,
    )


def cmd_compare(args):
    results = read_tsv(args.results)
    return report(
        args.arch,
        results,
        Path(args.expected) if args.expected else None,
        args.update,
        args.strict,
        listed=listed_tests(
            args.root or REPO / "target" / f"rootfs-compat-{args.arch}", SUITES
        ),
    )


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    lp = sub.add_parser("lists", help="write the test lists and build.tsv")
    lp.add_argument("--arch", required=True)
    lp.add_argument("--ltp-src", required=True, help="the LTP build tree")
    lp.add_argument("--ltp-stage", required=True, help="the installed /opt/ltp")
    lp.add_argument("--make-log", help="LTP's make output (Open POSIX compile errors)")
    lp.add_argument("--removed", help="LTP sources left out: PATH<TAB>REASON per line")
    lp.add_argument("--libc-test", required=True, help="the built libc-test tree")
    lp.add_argument("--libc-test-log", help="libc-test's make output")
    lp.add_argument(
        "--root", required=True, help="the image's root (to resolve programs)"
    )
    lp.add_argument("--out", required=True, help="directory for the .list files")
    lp.add_argument("--build-tsv", required=True)
    lp.add_argument("--expected", help="expected list; its passes make the smoke lists")

    for name in ("run", "compare", "collect"):
        p = sub.add_parser(name)
        p.add_argument(
            "--arch", required=True, choices=("x86_64", "aarch64", "riscv64")
        )
        p.add_argument("--expected", help="default tests/compat/expected-<arch>.tsv")
        p.add_argument(
            "--update", action="store_true", help="rewrite the expected list"
        )
        p.add_argument("--strict", action="store_true", help="fail on any difference")
        if name == "compare":
            p.add_argument("results")
    for name in ("compare", "collect"):
        sub.choices[name].add_argument(
            "--root",
            help="the image's tree, for its lists (default target/rootfs-compat-<arch>)",
        )
    cp = sub.choices["collect"]
    cp.add_argument(
        "--out", help="default target/compat/<arch> (its serial-*.log files)"
    )
    cp.add_argument("logs", nargs="*", help="these logs instead, in this order")
    rp = sub.choices["run"]
    rp.add_argument("--image", help="default target/rootfs-compat-<arch>.img")
    rp.add_argument("--out", help="default target/compat/<arch>")
    rp.add_argument("--build-tsv", help="default target/compat/<arch>/build.tsv")
    rp.add_argument("--suites", help="comma-separated (default: all three)")
    rp.add_argument("--first", type=int, help="start the first suite at this test")
    rp.add_argument("--timeout", type=int, help="seconds per test (60 KVM, 180 TCG)")
    rp.add_argument(
        "--boot-wait", type=int, help="seconds for the prompt (300 KVM, 900 TCG)"
    )
    rp.add_argument(
        "--grace", type=int, help="seconds past the limit before a hang (90/300)"
    )
    rp.add_argument("--smp", type=int, default=1)
    rp.add_argument(
        "--tcg", action="store_true", help="x86_64 without KVM (TCG limits)"
    )
    rp.add_argument(
        "--retry",
        action="store_true",
        help="run a test that regressed once more; it counts only if it fails again",
    )
    rp.add_argument("--max-boots", type=int, default=200)
    rp.add_argument("--jobs", type=int, default=1, help="guests run side by side (TCG)")
    rp.add_argument(
        "--smoke", action="store_true", help="only the tests expected to pass"
    )
    rp.add_argument(
        "--resume",
        action="store_true",
        help="only the tests without a result in results.tsv",
    )
    rp.add_argument(
        "--root", help="the image's tree (default target/rootfs-compat-<arch>)"
    )

    args = ap.parse_args(argv)
    if args.cmd == "lists":
        cmd_lists(args)
        return 0
    if args.cmd == "run":
        return cmd_run(args)
    if args.cmd == "collect":
        return cmd_collect(args)
    return cmd_compare(args)


if __name__ == "__main__":
    sys.exit(main())
