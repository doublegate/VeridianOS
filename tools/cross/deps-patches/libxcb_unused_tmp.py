"""libxcb 1.17: no walk pointer in generated functions that never read it.

c_client.py generates the _sizeof, _unpack and _unserialize functions
with a pointer, xcb_tmp, that walks the wire buffer past each field. Many
never read it: they only add each field's length to it (and to the length
they return). GCC 16 no longer counts `xcb_tmp += n` as a use and reports
the variable as set but unused (-Wunused-but-set-variable), 276 times
across the generated sources.

The generator now emits the declaration after the function's code is
generated, when it knows whether any line reads xcb_tmp; when none does,
it emits neither the declaration nor the statements that only move
xcb_tmp. The functions' results are unchanged: the removed statements
updated nothing else. Idempotent.

Usage: libxcb_unused_tmp.py <libxcb source directory>
"""

import sys
from pathlib import Path

path = Path(sys.argv[1]) / "src" / "c_client.py"
text = path.read_text()
MARK = "def _c_reads_xcb_tmp(line):"
if MARK in text:
    sys.exit(0)


def replace_once(old, new):
    global text
    if text.count(old) != 1:
        sys.exit(
            f"{path}: expected one match for {old.strip()[:60]!r}, found {text.count(old)}"
        )
    text = text.replace(old, new)


# The helpers, before the serializer.
replace_once(
    "def _c_serialize(context, self):\n",
    "def _c_reads_xcb_tmp(line):\n"
    "    '''\n"
    "    Whether a generated line reads xcb_tmp (rather than only setting it,\n"
    "    as `xcb_tmp += n;` and `xcb_tmp = expr;` do).\n"
    "    '''\n"
    "    code = line.strip().replace('xcb_tmp_len', '')\n"
    "    if 'xcb_tmp' not in code:\n"
    "        return False\n"
    "    m = re.match(r'xcb_tmp\\s*\\+?=(?!=)(.*);$', code)\n"
    "    return m is None or 'xcb_tmp' in m.group(1)\n"
    "\n"
    "def _c_only_moves_xcb_tmp(line):\n"
    "    code = line.strip().replace('xcb_tmp_len', '')\n"
    "    return 'xcb_tmp' in code and not _c_reads_xcb_tmp(line)\n"
    "\n"
    "def _c_serialize(context, self):\n",
)

# unserialize and unpack: the declaration is deferred.
replace_once(
    "    elif context in ('unserialize', 'unpack'):\n"
    "        _c('    char *xcb_tmp = (char *)_buffer;')\n",
    "    elif context in ('unserialize', 'unpack'):\n"
    "        tmp_decl = '    char *xcb_tmp = (char *)_buffer;'\n",
)
# sizeof: likewise.
replace_once(
    "        else:\n"
    "            _c('    char *xcb_tmp = (char *)_buffer;')\n"
    "            prefix = [('_aux', '->', self)]\n",
    "        else:\n"
    "            tmp_decl = '    char *xcb_tmp = (char *)_buffer;'\n"
    "            prefix = [('_aux', '->', self)]\n",
)
# Initialized with the other locals.
replace_once(
    "    _c_pre.redirect_start(code_lines, temp_vars)\n",
    "    tmp_decl = None\n    _c_pre.redirect_start(code_lines, temp_vars)\n",
)
# Emitted (or not) once the code is known. unserialize reads it afterwards
# (to move the variable-size fields), so it always keeps it.
replace_once(
    "    _c_pre.redirect_end()\n\n    _c('')\n    for t in temp_vars:\n        _c(t)\n",
    "    _c_pre.redirect_end()\n"
    "\n"
    "    if tmp_decl is not None:\n"
    "        if context == 'unserialize' or any(_c_reads_xcb_tmp(l) for l in code_lines):\n"
    "            _c(tmp_decl)\n"
    "        else:\n"
    "            code_lines = [l for l in code_lines if not _c_only_moves_xcb_tmp(l)]\n"
    "\n"
    "    _c('')\n"
    "    for t in temp_vars:\n"
    "        _c(t)\n",
)
if "\nimport re\n" not in text and "\nimport re," not in text:
    replace_once("import getopt\n", "import getopt\nimport re\n")
path.write_text(text)
