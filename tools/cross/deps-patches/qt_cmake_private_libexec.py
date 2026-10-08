"""Qt 6.12: qt-cmake-private finds Qt's toolchain file from its libexec dir.

qt-cmake-private is generated from bin/qt-cmake.in, whose toolchain path is
relative to the bin directory, but it is installed in the libexec
directory. The two agree only when libexec is one level below the prefix;
with -archdatadir lib/qt6 (libexec in lib/qt6/libexec) it pointed at
lib/qt6/lib/cmake, so qt-configure-module could configure no module.
The path is now computed from the libexec directory for that script.

Usage: qt_cmake_private_libexec.py <cmake/QtWrapperScriptHelpers.cmake>
"""

import sys
from pathlib import Path

path = sys.argv[1]
text = Path(path).read_text()
old = (
    "    if(generate_unix)\n"
    '        configure_file("${CMAKE_CURRENT_SOURCE_DIR}/bin/qt-cmake.in"\n'
    '            "${QT_BUILD_DIR}/${INSTALL_LIBEXECDIR}/qt-cmake-private" @ONLY\n'
    "            NEWLINE_STYLE LF)\n"
)
new = (
    "    if(generate_unix)\n"
    "        # Installed in the libexec directory: the toolchain path is\n"
    "        # relative to it, not to the bin directory.\n"
    "        set(__qt_bin_relative_cmake_dir\n"
    '            "${__GlobalConfig_relative_path_from_bin_dir_to_cmake_config_dir}")\n'
    "        file(RELATIVE_PATH __GlobalConfig_relative_path_from_bin_dir_to_cmake_config_dir\n"
    '            "/${INSTALL_LIBEXECDIR}" "/${__GlobalConfig_install_dir}")\n'
    '        configure_file("${CMAKE_CURRENT_SOURCE_DIR}/bin/qt-cmake.in"\n'
    '            "${QT_BUILD_DIR}/${INSTALL_LIBEXECDIR}/qt-cmake-private" @ONLY\n'
    "            NEWLINE_STYLE LF)\n"
    "        set(__GlobalConfig_relative_path_from_bin_dir_to_cmake_config_dir\n"
    '            "${__qt_bin_relative_cmake_dir}")\n'
)
if new not in text:
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for the qt-cmake-private wrapper")
    Path(path).write_text(text.replace(old, new))
