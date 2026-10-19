"""libnl 3.12: a --disable-unit-tests option.

The unit tests (make check) need the check library. configure looks for it
unconditionally and, without it, warns "*** Disabling building of unit
tests"; there is no option to say the tests are not wanted. This adds one,
--disable-unit-tests (default: enabled, as before), which skips the search
and leaves the tests out (has_check=no, the WITH_CHECK conditional off).
Both configure.ac and the generated configure are changed: the option is
registered in configure's list of known options (so it is not reported as
unrecognized), described in --help, and the check library search runs only
when it is enabled. Both keep their modification times. Idempotent.

Usage: libnl_unit_tests_option.py <libnl source directory>
"""

import os
import sys
from pathlib import Path


def write_keeping_mtime(path, text):
    """Write PATH, keeping its modification time: make compares the
    autotools files' times and would otherwise run aclocal, autoconf and
    automake again (versions the build host may not have)."""
    st = path.stat()
    path.write_text(text)
    os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns))


root = Path(sys.argv[1])


def replace_once(text, old, new, what):
    if text.count(old) != 1:
        sys.exit(f"{what}: expected one match, found {text.count(old)}")
    return text.replace(old, new)


# configure.ac
path = root / "configure.ac"
text = path.read_text()
if "unit-tests" not in text:
    old = (
        "PKG_CHECK_MODULES([CHECK], [check >= 0.9.0],\n"
        '\t[has_check="yes"],\n'
        "\t[AC_MSG_WARN([*** Disabling building of unit tests])\n"
        '\t has_check="no"])\n'
    )
    new = (
        "AC_ARG_ENABLE([unit-tests],\n"
        "\tAS_HELP_STRING([--disable-unit-tests],\n"
        "\t\t[Do not build the unit tests (they need the check library)]),\n"
        '\t[enable_unit_tests="$enableval"], [enable_unit_tests="yes"])\n'
        'AS_IF([test "x$enable_unit_tests" = xno], [has_check="no"],\n'
        "[PKG_CHECK_MODULES([CHECK], [check >= 0.9.0],\n"
        '\t[has_check="yes"],\n'
        "\t[AC_MSG_WARN([*** Disabling building of unit tests])\n"
        '\t has_check="no"])])\n'
    )
    write_keeping_mtime(path, replace_once(text, old, new, path))

# configure
path = root / "configure"
text = path.read_text()
if "enable_unit_tests" not in text:
    # Known option.
    text = replace_once(
        text,
        "ac_user_opts='\nenable_option_checking\n",
        "ac_user_opts='\nenable_option_checking\nenable_unit_tests\n",
        f"{path} (ac_user_opts)",
    )
    # --help: after the first optional feature, which every autoconf
    # configure lists.
    text = replace_once(
        text,
        "  --disable-option-checking  ignore unrecognized --enable/--with options\n",
        "  --disable-option-checking  ignore unrecognized --enable/--with options\n"
        "  --disable-unit-tests    Do not build the unit tests (they need the check\n"
        "                          library)\n",
        f"{path} (--help)",
    )
    # The search, skipped when disabled.
    start = (
        "pkg_failed=no\n"
        '{ printf "%s\\n" "$as_me:${as_lineno-$LINENO}: checking for check >= 0.9.0" >&5\n'
    )
    text = replace_once(
        text,
        start,
        "# Check whether --enable-unit-tests was given.\n"
        "if test ${enable_unit_tests+y}\n"
        "then :\n"
        '  enableval=$enable_unit_tests; enable_unit_tests="$enableval"\n'
        "else case e in #(\n"
        '  e) enable_unit_tests="yes" ;;\n'
        "esac\n"
        "fi\n"
        'if test "x$enable_unit_tests" = xno\n'
        "then :\n"
        '  has_check="no"\n'
        "else case e in #(\n"
        "  e) " + start,
        f"{path} (check search)",
    )
    end = '        has_check="yes"\nfi\n if test "$has_check" = \'yes\'; then\n'
    text = replace_once(
        text,
        end,
        '        has_check="yes"\n'
        "fi ;;\n"
        "esac\n"
        "fi\n"
        " if test \"$has_check\" = 'yes'; then\n",
        f"{path} (end of check search)",
    )
    write_keeping_mtime(path, text)
