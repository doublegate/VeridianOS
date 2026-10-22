#!/usr/bin/env python3
"""Unit tests for tools/compat/compat.py (run by scripts/dev/gate.sh)."""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import compat


class Classify(unittest.TestCase):
    def test_ltp_exit_codes(self):
        c = compat.classify
        self.assertEqual(c("ltp-syscalls", "rc=0 sig=0 timeout=0 ms=5")[0], "PASS")
        self.assertEqual(c("ltp-syscalls", "rc=1 sig=0 timeout=0 ms=5")[0], "FAIL")
        # TBROK wins over TFAIL; TCONF alone is a skip.
        self.assertEqual(c("ltp-syscalls", "rc=3 sig=0 timeout=0 ms=5")[0], "BROK")
        self.assertEqual(c("ltp-syscalls", "rc=32 sig=0 timeout=0 ms=5")[0], "CONF")
        self.assertEqual(c("ltp-syscalls", "rc=33 sig=0 timeout=0 ms=5")[0], "FAIL")
        self.assertEqual(c("ltp-syscalls", "rc=4 sig=0 timeout=0 ms=5")[0], "WARN")

    def test_open_posix_exit_codes(self):
        c = compat.classify
        self.assertEqual(c("open-posix", "rc=0 sig=0 timeout=0 ms=1")[0], "PASS")
        self.assertEqual(c("open-posix", "rc=1 sig=0 timeout=0 ms=1")[0], "FAIL")
        self.assertEqual(c("open-posix", "rc=2 sig=0 timeout=0 ms=1")[0], "BROK")
        self.assertEqual(c("open-posix", "rc=4 sig=0 timeout=0 ms=1")[0], "CONF")
        self.assertEqual(c("open-posix", "rc=5 sig=0 timeout=0 ms=1")[0], "CONF")
        self.assertEqual(c("open-posix", "rc=9 sig=0 timeout=0 ms=1")[0], "FAIL")

    def test_libc_test(self):
        self.assertEqual(
            compat.classify("libc-test", "rc=0 sig=0 timeout=0 ms=1")[0], "PASS"
        )
        self.assertEqual(
            compat.classify("libc-test", "rc=1 sig=0 timeout=0 ms=1")[0], "FAIL"
        )

    def test_common_outcomes(self):
        c = compat.classify
        self.assertEqual(
            c("libc-test", "rc=-1 sig=9 timeout=1 ms=60000"), ("TIMEOUT", 60000, "")
        )
        self.assertEqual(
            c("libc-test", "rc=-1 sig=11 timeout=0 ms=3"), ("SIGNAL", 3, "SIGSEGV")
        )
        self.assertEqual(c("open-posix", "rc=127 sig=0 timeout=0 ms=3")[0], "EXECFAIL")
        self.assertEqual(c("ltp-syscalls", "notbuilt"), ("NOTBUILT", 0, ""))
        self.assertEqual(c("ltp-syscalls", "garbage")[0], "CRASH")


SERIAL = """\
root@veridian:/# /bin/ash /usr/lib/compat/run.sh /usr/lib/compat/ltp-syscalls.list 1 60
COMPAT-BEGIN 1 abort01
COMPAT-END 1 abort01 rc=0 sig=0 timeout=0 ms=120
COMPAT-BEGIN 2 accept01
COMPAT-LOG 2 accept01.c:42: TFAIL: accept() succeeded
COMPAT-LOG 2 second line
COMPAT-END 2 accept01 rc=1 sig=0 timeout=0 ms=80
COMPAT-BEGIN 3 accept02
"""


class Parse(unittest.TestCase):
    def test_events(self):
        p = compat.SerialParser()
        p.feed(SERIAL)
        self.assertEqual([r.id for r in p.done], ["abort01", "accept01"])
        self.assertEqual(
            p.done[1].logs, ["accept01.c:42: TFAIL: accept() succeeded", "second line"]
        )
        self.assertEqual(p.current, (3, "accept02"))
        self.assertIsNone(p.summary)

    def test_partial_lines_wait_for_newline(self):
        p = compat.SerialParser()
        p.feed("COMPAT-BEGIN 1 a\nCOMPAT-END 1 a rc=0 si")
        self.assertEqual(p.done, [])
        p.feed("g=0 timeout=0 ms=1\nCOMPAT-SUMMARY /x.list 1\n")
        self.assertEqual(len(p.done), 1)
        self.assertEqual(p.summary, 1)
        self.assertIsNone(p.current)

    def test_test_output_is_not_a_prompt_or_a_panic(self):
        # A test's own output is copied into COMPAT-LOG lines; text there
        # must not look like the shell coming back or the kernel panicking.
        p = compat.SerialParser()
        p.feed(
            "COMPAT-BEGIN 1 a\nCOMPAT-LOG 1 echo root@veridian:/# done\nCOMPAT-LOG 1 it panicked at x\n"
        )
        self.assertEqual((p.prompts, p.panic), (0, False))
        p.feed("COMPAT-END 1 a rc=1 sig=0 timeout=0 ms=1\nroot@veridian:/# ")
        self.assertEqual(p.prompts, 1)
        p.feed("\n[ 9.1] panicked at mm.rs:10\n")
        self.assertEqual((p.prompts, p.panic), (1, True))

    def test_a_prompt_split_between_pieces_counts_once(self):
        p = compat.SerialParser()
        p.feed("root@veri")
        p.feed("dian:/# ")
        self.assertEqual(p.prompts, 1)
        p.feed("/bin/ash x\n")
        self.assertEqual(p.prompts, 1)

    def test_carriage_returns_and_noise(self):
        p = compat.SerialParser()
        p.feed(
            "\x1b[0mjunk COMPAT-BEGIN 1 a\r\nCOMPAT-END 1 a rc=0 sig=0 timeout=0 ms=1\r\n"
        )
        self.assertEqual([r.id for r in p.done], ["a"])

    def test_note_prefers_failure_lines(self):
        r = compat.Record(
            2, "x", "rc=1 sig=0 timeout=0 ms=1", ["info", "x.c:3: TBROK: no"]
        )
        self.assertEqual(compat.note_for(r), "x.c:3: TBROK: no")
        r = compat.Record(2, "x", "rc=1 sig=0 timeout=0 ms=1", ["only\tline"])
        self.assertEqual(compat.note_for(r), "only line")

    def test_note_drops_terminal_escapes(self):
        # A child exec'd with an empty environment colours its output.
        r = compat.Record(
            1,
            "a",
            "rc=1 sig=0 timeout=0 ms=1",
            ["c.c:15: \x1b[1;31mTFAIL: \x1b[0margc is 0\x07"],
        )
        self.assertEqual(compat.note_for(r, status="FAIL"), "c.c:15: TFAIL: argc is 0")

    def test_note_hides_ltp_temporary_names(self):
        r = compat.Record(
            1,
            "a",
            "rc=2 sig=0 timeout=0 ms=1",
            ["x: statfs(/tmp/LTP_aboceFCgD): ENOSYS"],
        )
        self.assertEqual(
            compat.note_for(r, status="BROK"), "x: statfs(/tmp/LTP_XXXXXX): ENOSYS"
        )

    def test_note_matches_the_status(self):
        logs = [
            "a.c:1: TWARN: cleanup failed",
            "a.c:2: TBROK: statfs: ENOSYS",
            "a.c:3: TCONF: no",
        ]
        r = compat.Record(2, "a", "rc=6 sig=0 timeout=0 ms=1", logs)
        self.assertEqual(
            compat.note_for(r, status="BROK"), "a.c:2: TBROK: statfs: ENOSYS"
        )
        self.assertEqual(compat.note_for(r, status="CONF"), "a.c:3: TCONF: no")
        self.assertEqual(
            compat.note_for(r, status="WARN"), "a.c:1: TWARN: cleanup failed"
        )


class Compare(unittest.TestCase):
    def test_regressions_and_improvements(self):
        expected = {
            ("s", "a"): ("PASS", ""),
            ("s", "b"): ("FAIL", "known: N-1"),
            ("s", "c"): ("PASS", ""),
            ("s", "gone"): ("PASS", ""),
            ("s", "f"): ("FLAKY", "sometimes times out"),
        }
        actual = {
            ("s", "a"): ("PASS", ""),
            ("s", "b"): ("PASS", ""),
            ("s", "c"): ("TIMEOUT", ""),
            ("s", "new"): ("FAIL", ""),
            ("s", "f"): ("TIMEOUT", ""),
        }
        d = compat.compare(expected, actual)
        self.assertEqual(d["regressed"], [("s", "c", "PASS", "TIMEOUT")])
        self.assertEqual(d["improved"], [("s", "b", "FAIL", "PASS")])
        self.assertEqual(d["missing"], [("s", "gone")])
        self.assertEqual(d["new"], [("s", "new", "FAIL")])
        self.assertEqual(d["changed"], [])

    def test_an_expected_pass_that_did_not_run_fails_a_full_run(self):
        expected = {("s", "a"): ("PASS", ""), ("s", "b"): ("FAIL", "")}
        d = compat.compare(expected, {})
        self.assertEqual(
            compat.failures(d, expected, strict=False, subset=False), [("s", "a")]
        )
        # A smoke run is a subset: it compares only what it ran.
        self.assertEqual(compat.failures(d, expected, strict=False, subset=True), [])

    def test_update_needs_a_list_for_every_suite(self):
        # No list for a suite means its coverage is unknown, not complete.
        self.assertEqual(
            compat.uncovered({("s", "a"): ("PASS", "")}, {}), [("s", "(no list)")]
        )
        self.assertEqual(compat.uncovered({("s", "a"): ("PASS", "")}, {"s": {"a"}}), [])

    def test_update_needs_every_listed_test(self):
        listed = {"s": {"a", "b"}, "t": {"c"}}
        self.assertEqual(
            compat.uncovered({("s", "a"): ("PASS", "")}, listed), [("s", "b")]
        )
        self.assertEqual(
            compat.uncovered(
                {("s", "a"): ("PASS", ""), ("s", "b"): ("FAIL", "")}, listed
            ),
            [],
        )

    def test_other_changes(self):
        d = compat.compare({("s", "a"): ("FAIL", "")}, {("s", "a"): ("TIMEOUT", "")})
        self.assertEqual(d["changed"], [("s", "a", "FAIL", "TIMEOUT")])
        self.assertEqual(d["regressed"], [])

    def test_update_keeps_notes_when_status_holds(self):
        expected = {
            ("s", "a"): ("FAIL", "known: docs/KNOWN-LIMITATIONS.md"),
            ("s", "b"): ("FAIL", "old"),
        }
        actual = {
            ("s", "a"): ("FAIL", "auto"),
            ("s", "b"): ("BROK", "new reason"),
            ("s", "c"): ("PASS", ""),
        }
        merged = compat.merge_expected(expected, actual)
        self.assertEqual(
            merged[("s", "a")], ("FAIL", "known: docs/KNOWN-LIMITATIONS.md")
        )
        self.assertEqual(merged[("s", "b")], ("BROK", "new reason"))
        self.assertEqual(merged[("s", "c")], ("PASS", ""))

    def test_update_keeps_flaky_entries(self):
        merged = compat.merge_expected(
            {("s", "f"): ("FLAKY", "why")}, {("s", "f"): ("PASS", "")}
        )
        self.assertEqual(merged[("s", "f")], ("FLAKY", "why"))

    def test_tsv_round_trip(self):
        rows = {
            ("ltp-syscalls", "read01"): ("PASS", ""),
            ("libc-test", "math/sin"): ("FAIL", "a b"),
        }
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "e.tsv")
            compat.write_tsv(path, rows, "x86_64")
            self.assertEqual(compat.read_tsv(path), rows)


class Collect(unittest.TestCase):
    def test_results_from_serial_logs(self):
        log1 = (
            "root@veridian:/# /bin/ash /usr/lib/compat/run.sh /usr/lib/compat/ltp-syscalls.list 1 60\n"
            "[SHELL-EXEC] command_line='/bin/ash /usr/lib/compat/run.sh /usr/lib/compat/ltp-syscalls.list 1 60'\n"
            "COMPAT-BEGIN 1 a\nCOMPAT-END 1 a rc=0 sig=0 timeout=0 ms=1\n"
            "COMPAT-SUMMARY /usr/lib/compat/ltp-syscalls.list 1\n"
            "root@veridian:/# /bin/ash /usr/lib/compat/run.sh /usr/lib/compat/libc-test-smoke.list 1 60\n"
            "COMPAT-BEGIN 1 math/sin\nCOMPAT-LOG 1 bad\nCOMPAT-END 1 math/sin rc=1 sig=0 timeout=0 ms=1\n"
            "COMPAT-BEGIN 2 math/cos\n[ 1.0] panicked at mm.rs\n"
        )
        log2 = "x /usr/lib/compat/run.sh /usr/lib/compat/libc-test.list 3 60\nCOMPAT-BEGIN 3 math/tan\n"
        r = compat.results_from_logs([log1, log2])
        self.assertEqual(r[("ltp-syscalls", "a")], ("PASS", ""))
        self.assertEqual(r[("libc-test", "math/sin")], ("FAIL", "bad"))
        self.assertEqual(r[("libc-test", "math/cos")][0], "CRASH")
        self.assertEqual(r[("libc-test", "math/tan")][0], "HANG")


class Smoke(unittest.TestCase):
    def test_smoke_lines_are_the_expected_passes_renumbered(self):
        lines = ["1 a - /x/a", "2 b - /x/b -i 1", "3 c - -", "4 d - /x/d"]
        expected = {
            ("s", "b"): ("PASS", ""),
            ("s", "c"): ("PASS", ""),
            ("s", "d"): ("FAIL", ""),
        }
        self.assertEqual(
            compat.smoke_lines(lines, "s", expected), ["1 b - /x/b -i 1", "2 c - -"]
        )

    def test_suite_of_a_smoke_list(self):
        self.assertEqual(compat.suite_of("ltp-syscalls-smoke"), "ltp-syscalls")
        self.assertEqual(compat.suite_of("libc-test"), "libc-test")

    def test_compare_subset_ignores_tests_not_run(self):
        expected = {("s", "a"): ("PASS", ""), ("s", "b"): ("PASS", "")}
        d = compat.compare(
            compat.only_run(expected, {("s", "a"): ("PASS", "")}),
            {("s", "a"): ("PASS", "")},
        )
        self.assertEqual(d["missing"], [])


class Stall(unittest.TestCase):
    def test_a_guest_still_writing_is_given_longer(self):
        # limit 60, grace 90: silent past 150 s is a hang; talking, 420 s.
        self.assertFalse(compat.stalled(100, 0, 0, 60, 90))
        self.assertTrue(compat.stalled(160, 0, 100, 60, 90))
        self.assertFalse(compat.stalled(160, 0, 150, 60, 90))
        self.assertFalse(compat.stalled(400, 0, 395, 60, 90))
        self.assertTrue(compat.stalled(430, 0, 425, 60, 90))


class Ranges(unittest.TestCase):
    def test_split_covers_every_test_once(self):
        self.assertEqual(compat.split_ranges(10, 3), [(1, 4), (5, 7), (8, 10)])
        self.assertEqual(compat.split_ranges(2, 4), [(1, 1), (2, 2)])
        self.assertEqual(compat.split_ranges(5, 1), [(1, 5)])
        for count, jobs in ((1552, 6), (481, 7), (1770, 4)):
            ranges = compat.split_ranges(count, jobs)
            covered = [n for first, last in ranges for n in range(first, last + 1)]
            self.assertEqual(covered, list(range(1, count + 1)))


class Resume(unittest.TestCase):
    def test_missing_ranges(self):
        lines = ["1 a - -", "2 b - -", "3 c - -", "4 d - -", "5 e - -", "6 f - -"]
        have = {("s", "a"), ("s", "d")}
        self.assertEqual(compat.missing_ranges(lines, "s", have), [(2, 3), (5, 6)])
        self.assertEqual(compat.missing_ranges(lines[:1], "s", have), [])

    def test_ranges_are_shared_out(self):
        shares = compat.share_ranges([("s", 1, 10), ("s", 20, 21), ("t", 1, 3)], 2)
        counts = [sum(last - first + 1 for _, first, last in share) for share in shares]
        self.assertEqual(sorted(counts), [7, 8])
        self.assertEqual(
            sorted(r for share in shares for r in share),
            [("s", 1, 8), ("s", 9, 10), ("s", 20, 21), ("t", 1, 3)],
        )


class Lists(unittest.TestCase):
    def test_runtest_entries(self):
        text = "#comment\n\nabort01 abort01\nexecve05 execve05 -i 5 -n 32\nsetdomainname01\tsetdomainname01\n"
        self.assertEqual(
            compat.runtest_entries(text),
            [
                ("abort01", "abort01"),
                ("execve05", "execve05 -i 5 -n 32"),
                ("setdomainname01", "setdomainname01"),
            ],
        )

    def test_list_lines_number_and_resolve(self):
        with tempfile.TemporaryDirectory() as root:
            for f in (
                "opt/ltp/testcases/bin/abort01",
                "opt/x/d/x_1-1.run-test",
                "opt/libc-test/src/f/a.exe",
            ):
                os.makedirs(os.path.dirname(os.path.join(root, f)), exist_ok=True)
                open(os.path.join(root, f), "w").close()
            entries = [
                ("abort01", "-", "abort01"),
                ("execve05", "-", "execve05 -i 5"),
                ("d/1-1", "/opt/x/d", "./x_1-1.run-test"),
                ("f/a", "/opt/libc-test", "src/f/a.exe"),
            ]
            self.assertEqual(
                compat.list_lines(entries, root),
                [
                    "1 abort01 - /opt/ltp/testcases/bin/abort01",
                    "2 execve05 - - -i 5",
                    "3 d/1-1 /opt/x/d /opt/x/d/x_1-1.run-test",
                    "4 f/a /opt/libc-test /opt/libc-test/src/f/a.exe",
                ],
            )

    def test_open_posix_entry(self):
        self.assertEqual(
            compat.open_posix_entry("./conformance/interfaces/aio_read/1-1.c"),
            (
                "conformance/interfaces/aio_read/1-1",
                "/opt/ltp/testcases/open_posix_testsuite/conformance/interfaces/aio_read",
                "./aio_read_1-1.run-test",
            ),
        )
        self.assertEqual(
            compat.open_posix_entry("./conformance/interfaces/clock_getres/2-1.sh")[2],
            "./clock_getres_2-1.sh",
        )
        # generate-makefiles.sh names a speculative/ test after its parent.
        self.assertEqual(
            compat.open_posix_entry(
                "./conformance/interfaces/clock_settime/speculative/4-3.c"
            )[2],
            "./clock_settime_speculative_4-3.run-test",
        )
        self.assertEqual(
            compat.open_posix_prefix("conformance/definitions/sys/mman_h"), "mman_h"
        )

    def test_open_posix_built_only_where_make_goes(self):
        # Upstream's Makefiles enter only directories with a Makefile, at
        # every level: conformance/definitions/sys has none, so nothing
        # under it is built or run by LTP either.
        with tempfile.TemporaryDirectory() as top:
            for d in (
                "conformance",
                "conformance/definitions",
                "conformance/definitions/aio_h",
                "conformance/definitions/sys/mman_h",
            ):
                os.makedirs(os.path.join(top, d), exist_ok=True)
                open(os.path.join(top, d, "Makefile"), "w").close()
            self.assertTrue(compat.make_reaches(top, "conformance/definitions/aio_h"))
            self.assertFalse(
                compat.make_reaches(top, "conformance/definitions/sys/mman_h")
            )

    def test_open_posix_compile_errors(self):
        log = (
            "../../../conformance/interfaces/pthread_rwlock_rdlock/2-1.c:151:9: error: implicit "
            "declaration of function 'pthread_rwlockattr_setkind_np'\n"
            "../../../conformance/interfaces/pthread_rwlock_rdlock/2-1.c:160:1: error: second\n"
        )
        self.assertEqual(
            compat.compile_errors(log),
            {
                "conformance/interfaces/pthread_rwlock_rdlock/2-1": "error: implicit declaration "
                "of function 'pthread_rwlockattr_setkind_np'"
            },
        )

    def test_open_posix_link_errors_go_with_the_verdict(self):
        log = (
            "conformance/interfaces/a/1-1 compile PASSED\n"
            "/x/bin/ld: /tmp/cc.o: in function `main':\n"
            "1-1.c:(.text+0x3d): undefined reference to `pthread_mutexattr_getprioceiling'\n"
            "collect2: error: ld returned 1 exit status\n"
            "conformance/interfaces/pthread_mutexattr_getprioceiling/1-1 compile FAILED; SKIPPING\n"
            "conformance/interfaces/b/1-1 compile FAILED; SKIPPING\n"
        )
        self.assertEqual(
            compat.compile_errors(log),
            {
                "conformance/interfaces/pthread_mutexattr_getprioceiling/1-1": "undefined reference "
                "to `pthread_mutexattr_getprioceiling'"
            },
        )


if __name__ == "__main__":
    unittest.main()
