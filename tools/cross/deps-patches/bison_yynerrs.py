"""Bison 3.8 yacc.c skeleton: the syntax error count used in pure parsers.

A pure parser generated from Bison's C skeleton (yacc.c) keeps the number
of syntax errors in a local variable, yynerrs, that grammar actions may
read; most grammars never do, and the parser itself only counts with it.
GCC (-Wunused-but-set-variable) then reports the variable in every such
parser it compiles (Solid's predicate parser, libnl's two grammars). The
skeleton's return path now marks it used with Bison's own YY_USE macro, as
it already does for other values a grammar may leave unread; a parser
with start-symbol entry points, which hands the count back, is unchanged.

Usage: bison_yynerrs.py <copy of Bison's data directory>
(the copy is used through BISON_PKGDATADIR; see lib/cross-env.sh)
"""

import sys
from pathlib import Path

path = Path(sys.argv[1]) / "skeletons" / "yacc.c"
text = path.read_text()
OLD = """m4_ifdef([b4_start_symbols], [[
  if (yyimpl)
    yyimpl->yynerrs = yynerrs;]])[
  return yyresult;"""
NEW = """m4_ifdef([b4_start_symbols], [[
  if (yyimpl)
    yyimpl->yynerrs = yynerrs;]], [[
  YY_USE (yynerrs);]])[
  return yyresult;"""
if NEW not in text:
    if text.count(OLD) != 1:
        sys.exit(f"{path}: the skeleton's return path has changed")
    path.write_text(text.replace(OLD, NEW))
