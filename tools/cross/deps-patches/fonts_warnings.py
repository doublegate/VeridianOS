"""Source fixes for warnings in the font stack.

fontconfig 2.18.3:
- src/fcxml.c reports parse positions with XML_GetCurrentLineNumber(),
  which expat 2.9 deprecates for XML_GetCurrentLineNumber64()
  (-Wdeprecated-declarations). The 64-bit call is used with expat >= 2.9;
  older expat and the libxml2 backend keep the old name.
- src/meson.build preprocesses fcobjshash.gperf.h with cc.preprocess();
  meson does not count a header as a source, so the target is empty
  ("Build target preprocessor_0 has no sources"). The file becomes
  fcobjshash.gperf.c, as upstream fontconfig (main) has done.

HarfBuzz 14.6.0:
- src/hb-ot-metrics.cc comments out a macro with // lines that end in a
  backslash, which continues the comment onto the next line
  (-Wcomment). The block becomes one /* */ comment.

Usage: fonts_warnings.py fontconfig|harfbuzz <source dir>
"""

import os
import sys
from pathlib import Path

kind, src = sys.argv[1:3]


def patch(path, old, new):
    text = Path(path).read_text()
    if new in text and old not in text:
        return
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for {old.splitlines()[0]!r}")
    Path(path).write_text(text.replace(old, new))


if kind == "fontconfig":
    patch(
        f"{src}/src/fcxml.c",
        "#endif /* ENABLE_LIBXML2 */\n\n#ifdef _WIN32\n",
        "#endif /* ENABLE_LIBXML2 */\n\n"
        "/* expat 2.9 deprecates XML_GetCurrentLineNumber for the 64-bit call. */\n"
        "#if !defined(ENABLE_LIBXML2) && defined(XML_MAJOR_VERSION) && \\\n"
        "    (XML_MAJOR_VERSION > 2 || (XML_MAJOR_VERSION == 2 && XML_MINOR_VERSION >= 9))\n"
        "#  define FcXmlLineNumber(parser) XML_GetCurrentLineNumber64 (parser)\n"
        "#else\n"
        "#  define FcXmlLineNumber(parser) XML_GetCurrentLineNumber (parser)\n"
        "#endif\n\n#ifdef _WIN32\n",
    )
    path = f"{src}/src/fcxml.c"
    text = Path(path).read_text()
    calls = text.count("(int)XML_GetCurrentLineNumber (parse->parser)")
    if calls not in (0, 2):
        sys.exit(f"{path}: expected two line-number calls, found {calls}")
    Path(path).write_text(
        text.replace(
            "(int)XML_GetCurrentLineNumber (parse->parser)",
            "(int)FcXmlLineNumber (parse->parser)",
        )
    )
    old_h, new_c = f"{src}/src/fcobjshash.gperf.h", f"{src}/src/fcobjshash.gperf.c"
    if os.path.exists(old_h):
        os.rename(old_h, new_c)
    patch(
        f"{src}/src/meson.build",
        "cc.preprocess('fcobjshash.gperf.h',",
        "cc.preprocess('fcobjshash.gperf.c',",
    )
elif kind == "harfbuzz":
    patch(
        f"{src}/src/hb-ot-metrics.cc",
        "// Unused:\n"
        "//#define GET_METRIC_X(TABLE, ATTR) \\\n"
        "//  (face->table.TABLE->has_data () && \\\n"
        "//    ((void) (position && (*position = font->em_scalef_x (_fix_ascender_descender ( \\\n"
        "//      face->table.TABLE->ATTR + GET_VAR, metrics_tag)))), true))\n",
        "/* Unused:\n"
        "#define GET_METRIC_X(TABLE, ATTR) \\\n"
        "  (face->table.TABLE->has_data () && \\\n"
        "    ((void) (position && (*position = font->em_scalef_x (_fix_ascender_descender ( \\\n"
        "      face->table.TABLE->ATTR + GET_VAR, metrics_tag)))), true))\n"
        "*/\n",
    )
else:
    sys.exit(f"unknown package {kind}")
