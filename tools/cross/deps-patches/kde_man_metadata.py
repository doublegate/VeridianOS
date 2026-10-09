"""KDE manual pages: author metadata the DocBook man stylesheets accept.

attribution KDOCTOOLS_SRC DOCBOOK_XSL_DIR
    The DocBook man stylesheets (manpages/info.xsl, template "attribution")
    render a person in the AUTHOR section with their <contrib> or blurb,
    and otherwise with their role's localized label -- "Author." for an
    <author>, "Editor." for an <editor> -- after warning "AUTHOR sect.: no
    personblurb|contrib for <name>". KDE's manual pages credit their
    authors as authors, without a <contrib>, so every page and every
    translation warned. KDocTools' man stylesheet (kde-include-man.xsl)
    now carries the same template with the warning kept only where the
    role cannot be shown (anything but an author, an editor, or an
    <othercredit> whose class is set and not "other"); the pages
    come out byte-identical. The template is taken from DOCBOOK_XSL_DIR,
    so it matches the stylesheets it overrides.

corpauthor SRC FILE...
    A manual page with no author at all (meinproc6, kpackagetool6) gets
    "[FIXME: author]" in its header and the warning "meta author: no
    refentry/info/author". Those pages already name their product (KDE
    Frameworks); they now name its maker as corporate author,
    <corpauthor>The KDE Community</corpauthor>, in the English page and
    every translation of it found under SRC.

Idempotent.
"""

import re
import sys
from pathlib import Path

MARK = "<!-- VeridianOS: attribution -->"


def attribution(src, docbook):
    info = (Path(docbook) / "manpages" / "info.xsl").read_text()
    m = re.search(
        r'  <xsl:template name="attribution">.*?\n  </xsl:template>\n', info, re.DOTALL
    )
    if not m:
        sys.exit(f"{docbook}: no attribution template in manpages/info.xsl")
    old = '          <xsl:if test="$refentry.meta.get.quietly = 0">'
    new = (
        '          <xsl:if test="$refentry.meta.get.quietly = 0 and '
        "not(self::author or self::editor or "
        "(self::othercredit and @class and @class != 'other'))\">"
    )
    template = m.group(0)
    if template.count(old) != 1:
        sys.exit(f"{docbook}: the attribution template has changed")
    template = template.replace(old, new)
    path = Path(src) / "src" / "customization" / "kde-include-man.xsl.cmake"
    text = path.read_text()
    if MARK in text:
        return
    block = (
        f"\n   {MARK}\n"
        "   <!-- DocBook's attribution template (manpages/info.xsl), warning only\n"
        "        where a person's role cannot be shown (anything but an author, an\n"
        "        editor, or an othercredit with a class other than 'other'): an\n"
        "        author or editor without a contrib is shown as Author or Editor,\n"
        "        which is what KDE means. -->\n"
        f"{template}"
    )
    if text.count("</xsl:stylesheet>") != 1:
        sys.exit(f"{path}: expected one </xsl:stylesheet>")
    path.write_text(text.replace("</xsl:stylesheet>", block + "\n</xsl:stylesheet>"))


def corpauthor(src, names):
    root = Path(src)
    count = 0
    for name in names:
        for page in root.rglob(name):
            text = page.read_text()
            info = re.search(r"<refentryinfo>(.*?)</refentryinfo>", text, re.DOTALL)
            if not info:
                sys.exit(f"{page}: no refentryinfo")
            body = re.sub(r"<!--.*?-->", "", info.group(1), flags=re.DOTALL)
            if re.search(r"<(author|authorgroup|corpauthor)\b", body):
                continue
            page.write_text(
                text.replace(
                    "<refentryinfo>",
                    "<refentryinfo>\n<corpauthor>The KDE Community</corpauthor>",
                    1,
                )
            )
            count += 1
        if not any(root.rglob(name)):
            sys.exit(f"{src}: no {name}")
    return count


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "attribution":
        attribution(sys.argv[2], sys.argv[3])
    elif mode == "corpauthor":
        corpauthor(sys.argv[2], sys.argv[3:])
    else:
        sys.exit(f"unknown mode {mode}")
