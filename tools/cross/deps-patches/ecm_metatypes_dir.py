"""ECM: honour KDE_INSTALL_QTMETATYPESDIR given on the command line.

With KDE_INSTALL_USE_QT_SYS_PATHS, KDEInstallDirs6 asks Qt's qtpaths for
Qt's directories. Cross-compiling, that is the host Qt's qtpaths, which
answers with the host Qt's directories, so the frameworks installed their
QML modules, plugins and metatypes there. The Qt/KDE phases therefore turn
it off and pass the target Qt's layout (lib/qt6/...) as KDE_INSTALL_*DIR
cache variables. Every directory can be set that way except the metatypes
one, which the else-branch defines as a plain variable ("metatypes", under
the prefix). It now keeps a value given on the command line.

Usage: ecm_metatypes_dir.py <kde-modules/KDEInstallDirs6.cmake>
"""

import sys
from pathlib import Path

path = sys.argv[1]
text = Path(path).read_text()
old = (
    '    _define_relative(QMLDIR LIBDIR "qml"\n'
    '        "QtQuick2 imports")\n'
    "\n"
    '    _define_non_cache(QTMETATYPESDIR "metatypes")\n'
    "endif()\n"
)
new = (
    '    _define_relative(QMLDIR LIBDIR "qml"\n'
    '        "QtQuick2 imports")\n'
    "\n"
    "    if(DEFINED CACHE{KDE_INSTALL_QTMETATYPESDIR})\n"
    '        _define_non_cache(QTMETATYPESDIR "$CACHE{KDE_INSTALL_QTMETATYPESDIR}")\n'
    "    else()\n"
    '        _define_non_cache(QTMETATYPESDIR "metatypes")\n'
    "    endif()\n"
    "endif()\n"
)
if new not in text:
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for the metatypes directory")
    Path(path).write_text(text.replace(old, new))
