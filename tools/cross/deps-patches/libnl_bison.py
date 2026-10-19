"""libnl 3.12: grammars without Bison's deprecated directives.

The two grammars use `%error-verbose` and `%name-prefix "x_"`, which Bison
3 deprecates (-Wdeprecated) in favour of `%define parse.error verbose` and
`%define api.prefix {x_}`. api.prefix also renames the semantic-value type
the grammar header exports, YYSTYPE, to X_STYPE; the flex lexers, which
take it through %option bison-bridge, get `#define YYSTYPE X_STYPE` after
including the header, and the one with locations (pktloc, which flex
reads through %option bison-locations) also `#define YYLTYPE X_LTYPE`;
so do the C files that include the lexers' headers (ematch.c, pktloc.c).

Usage: libnl_bison.py <libnl source directory>
"""

import sys
from pathlib import Path

root = Path(sys.argv[1])
fixes = [
    ("lib/route/cls/ematch_syntax.y", "ematch_"),
    ("lib/route/pktloc_syntax.y", "pktloc_"),
]


def replace_once(path, old, new):
    text = path.read_text()
    if new in text:
        return
    if text.count(old) != 1:
        sys.exit(f"{path}: expected one {old!r}")
    path.write_text(text.replace(old, new))


for grammar, prefix in fixes:
    y = root / grammar
    replace_once(y, "%error-verbose\n", "%define parse.error verbose\n")
    replace_once(y, f'%name-prefix "{prefix}"\n', f"%define api.prefix {{{prefix}}}\n")
    defines = f" #define YYSTYPE {prefix.upper()}STYPE\n"
    if "%locations" in y.read_text():
        defines += f" #define YYLTYPE {prefix.upper()}LTYPE\n"
    lexer = y.with_name(y.name.replace("_syntax.y", "_grammar.l"))
    header = y.name.replace(".y", ".h")
    replace_once(
        lexer,
        f' #include "{header}"\n',
        f' #include "{header}"\n{defines}',
    )
    # The C file that includes the lexer's header (ematch.c, pktloc.c):
    # its declarations use YYSTYPE (and YYLTYPE) too.
    user = y.with_name(y.name.replace("_syntax.y", ".c"))
    replace_once(
        user,
        f'#include "{header}"\n',
        f'#include "{header}"\n{defines.replace(" #define", "#define")}',
    )
