#!/bin/sh
# VeridianOS -- build-kde-rootfs.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build script for the KDE-enabled VeridianOS root filesystem image.
#
# Produces a 2 GB ext4 rootfs image containing:
#   - Base VeridianOS userland (libc, coreutils, shell, init)
#   - Qt 6 libraries
#   - KDE Frameworks 6 libraries
#   - KWin compositor
#   - Plasma Desktop (plasmashell, breeze, settings)
#   - Core KDE applications (Dolphin, Konsole, Kate, Spectacle)
#   - XWayland server
#   - D-Bus system and session configuration
#   - Font files and fontconfig cache
#   - Breeze theme assets (icons, cursors, wallpapers)
#   - Session configuration (/etc/veridian/session.conf)
#
# Usage:
#   ./build-kde-rootfs.sh [output-image]
#
# Environment:
#   VERIDIAN_SYSROOT    - Source sysroot (default: /opt/veridian-sysroot)
#   BASE_ROOTFS         - Base VeridianOS rootfs image
#                         (default: target/rootfs-blockfs.img)
#   ROOTFS_SIZE_MB      - Image size in MB (default: 2048)
#   DEFAULT_SESSION     - Default session type: builtin or plasma
#                         (default: plasma)

set -e

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
VERIDIAN_SYSROOT="${VERIDIAN_SYSROOT:-/opt/veridian-sysroot}"
BASE_ROOTFS="${BASE_ROOTFS:-${PROJECT_ROOT}/target/rootfs-blockfs.img}"
ROOTFS_SIZE_MB="${ROOTFS_SIZE_MB:-2048}"
DEFAULT_SESSION="${DEFAULT_SESSION:-plasma}"
OUTPUT_IMAGE="${1:-${PROJECT_ROOT}/target/rootfs-kde.img}"
MOUNT_POINT="/tmp/veridian-kde-rootfs"

echo "========================================"
echo "  KDE Root Filesystem Builder"
echo "========================================"
echo "  Sysroot:        ${VERIDIAN_SYSROOT}"
echo "  Base rootfs:    ${BASE_ROOTFS}"
echo "  Output:         ${OUTPUT_IMAGE}"
echo "  Size:           ${ROOTFS_SIZE_MB} MB"
echo "  Default session: ${DEFAULT_SESSION}"
echo "========================================"

# =========================================================================
# Step 1: Create rootfs image
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 1: Creating ${ROOTFS_SIZE_MB} MB rootfs image"
echo "================================================================"

dd if=/dev/zero of="${OUTPUT_IMAGE}" bs=1M count="${ROOTFS_SIZE_MB}" status=progress
mkfs.ext4 -F -L "VeridianOS-KDE" "${OUTPUT_IMAGE}"

echo "  Image created: ${OUTPUT_IMAGE}"

# =========================================================================
# Step 2: Mount and populate base system
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 2: Mounting and populating base system"
echo "================================================================"

mkdir -p "${MOUNT_POINT}"
mount -o loop "${OUTPUT_IMAGE}" "${MOUNT_POINT}"

# Trap to ensure cleanup on exit
cleanup() {
    umount "${MOUNT_POINT}" 2>/dev/null || true
    rmdir "${MOUNT_POINT}" 2>/dev/null || true
}
trap cleanup EXIT

# Create directory structure
mkdir -p "${MOUNT_POINT}/bin"
mkdir -p "${MOUNT_POINT}/sbin"
mkdir -p "${MOUNT_POINT}/usr/bin"
mkdir -p "${MOUNT_POINT}/usr/lib"
mkdir -p "${MOUNT_POINT}/usr/lib/qt6/plugins"
mkdir -p "${MOUNT_POINT}/usr/lib/qt6/qml"
mkdir -p "${MOUNT_POINT}/usr/lib/kf6"
mkdir -p "${MOUNT_POINT}/usr/libexec"
mkdir -p "${MOUNT_POINT}/usr/share/applications"
mkdir -p "${MOUNT_POINT}/usr/share/dbus-1/services"
mkdir -p "${MOUNT_POINT}/usr/share/dbus-1/system-services"
mkdir -p "${MOUNT_POINT}/usr/share/fonts/truetype"
mkdir -p "${MOUNT_POINT}/usr/share/icons"
mkdir -p "${MOUNT_POINT}/usr/share/themes"
mkdir -p "${MOUNT_POINT}/usr/share/wallpapers"
mkdir -p "${MOUNT_POINT}/usr/share/plasma"
mkdir -p "${MOUNT_POINT}/usr/share/kservices6"
mkdir -p "${MOUNT_POINT}/usr/share/X11/xkb"
mkdir -p "${MOUNT_POINT}/etc/dbus-1/system.d"
mkdir -p "${MOUNT_POINT}/etc/dbus-1/session.d"
mkdir -p "${MOUNT_POINT}/etc/veridian"
mkdir -p "${MOUNT_POINT}/etc/xdg"
mkdir -p "${MOUNT_POINT}/etc/fonts"
mkdir -p "${MOUNT_POINT}/run/dbus"
mkdir -p "${MOUNT_POINT}/run/user"
mkdir -p "${MOUNT_POINT}/tmp/.X11-unix"
mkdir -p "${MOUNT_POINT}/var/lib/dbus"

# Copy base rootfs contents if available
if [ -f "${BASE_ROOTFS}" ]; then
    echo "  Copying base rootfs..."
    BASE_MOUNT="/tmp/veridian-base-rootfs"
    mkdir -p "${BASE_MOUNT}"
    mount -o loop,ro "${BASE_ROOTFS}" "${BASE_MOUNT}" 2>/dev/null || true
    if mountpoint -q "${BASE_MOUNT}"; then
        cp -a "${BASE_MOUNT}"/* "${MOUNT_POINT}/" 2>/dev/null || true
        umount "${BASE_MOUNT}"
    fi
    rmdir "${BASE_MOUNT}" 2>/dev/null || true
    echo "  Base rootfs copied"
else
    echo "  WARNING: Base rootfs not found at ${BASE_ROOTFS}"
    echo "  Creating minimal directory structure only"
fi

# =========================================================================
# Step 3: Install Qt 6 libraries
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 3: Installing Qt 6 libraries"
echo "================================================================"

if [ -d "${VERIDIAN_SYSROOT}/usr/lib" ]; then
    # Core Qt libraries
    for LIB in Qt6Core Qt6Gui Qt6Widgets Qt6DBus Qt6Network Qt6Qml \
               Qt6Quick Qt6Svg Qt6WaylandClient Qt6WaylandCompositor \
               Qt6Xml Qt6Concurrent; do
        for FILE in "${VERIDIAN_SYSROOT}/usr/lib/lib${LIB}"*.so*; do
            if [ -f "${FILE}" ]; then
                cp -a "${FILE}" "${MOUNT_POINT}/usr/lib/"
            fi
        done
    done

    # Qt plugins
    if [ -d "${VERIDIAN_SYSROOT}/usr/lib/qt6/plugins" ]; then
        cp -a "${VERIDIAN_SYSROOT}/usr/lib/qt6/plugins"/* \
              "${MOUNT_POINT}/usr/lib/qt6/plugins/" 2>/dev/null || true
    fi

    # QML modules
    if [ -d "${VERIDIAN_SYSROOT}/usr/lib/qt6/qml" ]; then
        cp -a "${VERIDIAN_SYSROOT}/usr/lib/qt6/qml"/* \
              "${MOUNT_POINT}/usr/lib/qt6/qml/" 2>/dev/null || true
    fi

    echo "  Qt 6 libraries installed"
else
    echo "  WARNING: Sysroot not found at ${VERIDIAN_SYSROOT}"
fi

# =========================================================================
# Step 4: Install KDE Frameworks 6 libraries
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 4: Installing KDE Frameworks 6 libraries"
echo "================================================================"

for LIB in KF6ConfigCore KF6CoreAddons KF6I18n KF6KIOCore \
           KF6Service KF6WindowSystem KF6Solid KF6GlobalAccel \
           KF6Package KF6Plasma KF6Activities KF6Crash \
           KF6Declarative KF6Notifications KF6Wallet KF6Auth \
           KF6GuiAddons KF6WidgetsAddons KF6Completion KF6JobWidgets \
           KF6Bookmarks KF6ItemViews KF6DBusAddons KF6XmlGui \
           KF6TextWidgets KF6Sonnet KF6IconThemes; do
    for FILE in "${VERIDIAN_SYSROOT}/usr/lib/lib${LIB}"*.so*; do
        if [ -f "${FILE}" ]; then
            cp -a "${FILE}" "${MOUNT_POINT}/usr/lib/"
        fi
    done
done

echo "  KDE Frameworks 6 libraries installed"

# =========================================================================
# Step 5: Install KWin and Plasma binaries
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 5: Installing KWin and Plasma binaries"
echo "================================================================"

for BIN in kwin_wayland plasmashell kded6 kglobalaccel6 \
           kactivitymanagerd systemsettings6 \
           dolphin konsole kate spectacle \
           dbus-daemon dbus-send dbus-launch \
           plasma-veridian-session Xwayland \
           kbuildsycoca6 kquitapp6; do
    if [ -f "${VERIDIAN_SYSROOT}/usr/bin/${BIN}" ]; then
        cp -a "${VERIDIAN_SYSROOT}/usr/bin/${BIN}" \
              "${MOUNT_POINT}/usr/bin/"
    fi
done

# KWin plugins
if [ -d "${VERIDIAN_SYSROOT}/usr/lib/qt6/plugins/org.kde.kwin.platforms" ]; then
    mkdir -p "${MOUNT_POINT}/usr/lib/qt6/plugins/org.kde.kwin.platforms"
    cp -a "${VERIDIAN_SYSROOT}/usr/lib/qt6/plugins/org.kde.kwin.platforms"/* \
          "${MOUNT_POINT}/usr/lib/qt6/plugins/org.kde.kwin.platforms/" 2>/dev/null || true
fi

echo "  KWin and Plasma binaries installed"

# =========================================================================
# Step 6: Install Breeze theme assets
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 6: Installing Breeze theme assets"
echo "================================================================"

# Icons
if [ -d "${VERIDIAN_SYSROOT}/usr/share/icons/breeze" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/icons/breeze" \
          "${MOUNT_POINT}/usr/share/icons/"
fi
if [ -d "${VERIDIAN_SYSROOT}/usr/share/icons/breeze-dark" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/icons/breeze-dark" \
          "${MOUNT_POINT}/usr/share/icons/"
fi

# Cursors
if [ -d "${VERIDIAN_SYSROOT}/usr/share/icons/breeze_cursors" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/icons/breeze_cursors" \
          "${MOUNT_POINT}/usr/share/icons/"
fi

# Wallpapers
if [ -d "${VERIDIAN_SYSROOT}/usr/share/wallpapers" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/wallpapers"/* \
          "${MOUNT_POINT}/usr/share/wallpapers/" 2>/dev/null || true
fi

# Plasma desktop layouts and config
if [ -d "${VERIDIAN_SYSROOT}/usr/share/plasma" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/plasma"/* \
          "${MOUNT_POINT}/usr/share/plasma/" 2>/dev/null || true
fi

echo "  Breeze theme assets installed"

# =========================================================================
# Step 7: Install D-Bus configuration
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 7: Installing D-Bus configuration"
echo "================================================================"

# System bus config
cat > "${MOUNT_POINT}/etc/dbus-1/system.conf" << 'DBUSCONF'
<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=/run/dbus/system_bus_socket</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
  <includedir>/etc/dbus-1/system.d</includedir>
</busconfig>
DBUSCONF

# Session bus config
cat > "${MOUNT_POINT}/etc/dbus-1/session.conf" << 'DBUSCONF'
<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow own="*"/>
  </policy>
  <includedir>/etc/dbus-1/session.d</includedir>
</busconfig>
DBUSCONF

# Copy D-Bus service files
if [ -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/services" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/dbus-1/services"/* \
          "${MOUNT_POINT}/usr/share/dbus-1/services/" 2>/dev/null || true
fi
if [ -d "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/dbus-1/system-services"/* \
          "${MOUNT_POINT}/usr/share/dbus-1/system-services/" 2>/dev/null || true
fi

echo "  D-Bus configuration installed"

# =========================================================================
# Step 8: Install .desktop files
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 8: Installing .desktop files"
echo "================================================================"

if [ -d "${VERIDIAN_SYSROOT}/usr/share/applications" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/applications"/* \
          "${MOUNT_POINT}/usr/share/applications/" 2>/dev/null || true
fi

# Create session .desktop entries for the display manager
mkdir -p "${MOUNT_POINT}/usr/share/xsessions"
mkdir -p "${MOUNT_POINT}/usr/share/wayland-sessions"

cat > "${MOUNT_POINT}/usr/share/wayland-sessions/plasma-veridian.desktop" << 'DESKTOP'
[Desktop Entry]
Name=KDE Plasma 6 (VeridianOS)
Comment=KDE Plasma desktop on VeridianOS kernel
Exec=/usr/bin/plasma-veridian-session
Type=Application
DesktopNames=KDE
DESKTOP

cat > "${MOUNT_POINT}/usr/share/wayland-sessions/veridian-builtin.desktop" << 'DESKTOP'
[Desktop Entry]
Name=VeridianOS Desktop (built-in)
Comment=Lightweight built-in kernel compositor desktop
Exec=/bin/sh
Type=Application
DesktopNames=VeridianOS
DESKTOP

echo "  Desktop files installed"

# =========================================================================
# Step 9: Install fonts and generate cache
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 9: Installing fonts and fontconfig cache"
echo "================================================================"

# Copy fonts from sysroot
if [ -d "${VERIDIAN_SYSROOT}/usr/share/fonts" ]; then
    cp -a "${VERIDIAN_SYSROOT}/usr/share/fonts"/* \
          "${MOUNT_POINT}/usr/share/fonts/" 2>/dev/null || true
fi

# Fontconfig configuration
if [ -d "${VERIDIAN_SYSROOT}/etc/fonts" ]; then
    cp -a "${VERIDIAN_SYSROOT}/etc/fonts"/* \
          "${MOUNT_POINT}/etc/fonts/" 2>/dev/null || true
fi

# Generate fontconfig cache (if fc-cache is available for target)
if [ -x "${VERIDIAN_SYSROOT}/usr/bin/fc-cache" ]; then
    echo "  Generating fontconfig cache..."
    # Run under QEMU user-mode if needed (cross-compiled binary)
    "${VERIDIAN_SYSROOT}/usr/bin/fc-cache" -f \
        --sysroot="${MOUNT_POINT}" 2>/dev/null || true
elif command -v fc-cache >/dev/null 2>&1; then
    echo "  Generating fontconfig cache (host fc-cache)..."
    fc-cache -f -y "${MOUNT_POINT}" 2>/dev/null || true
else
    echo "  NOTE: fc-cache not available -- cache will be generated on first boot"
fi

echo "  Fonts installed"

# =========================================================================
# Step 10: Create session configuration
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 10: Creating session configuration"
echo "================================================================"

cat > "${MOUNT_POINT}/etc/veridian/session.conf" << SESSIONCONF
# VeridianOS session configuration
# Options: builtin, plasma
session_type=${DEFAULT_SESSION}
SESSIONCONF

# KWin default config
cat > "${MOUNT_POINT}/etc/xdg/kwinrc" << 'KWINRC'
[Compositing]
Backend=OpenGL
GLCore=false
GLVSync=true
AnimationSpeed=3

[Wayland]
InputMethod=
VirtualKeyboardEnabled=false

[Windows]
FocusPolicy=ClickToFocus

[Desktops]
Number=2
Rows=1
KWINRC

# KDE globals
cat > "${MOUNT_POINT}/etc/xdg/kdeglobals" << 'KDEGLOBALS'
[General]
ColorScheme=BreezeLight
Name=Breeze
widgetStyle=breeze

[Icons]
Theme=breeze

[KDE]
LookAndFeelPackage=org.kde.breeze.desktop
SingleClick=false
KDEGLOBALS

echo "  Session configuration created"

# =========================================================================
# Step 11: Install integration scripts
# =========================================================================

echo ""
echo "================================================================"
echo "  Step 11: Installing integration scripts"
echo "================================================================"

cp -f "${SCRIPT_DIR}/veridian-kde-init.sh" \
      "${MOUNT_POINT}/usr/libexec/veridian-kde-init"
chmod +x "${MOUNT_POINT}/usr/libexec/veridian-kde-init"

echo "  Integration scripts installed"

# =========================================================================
# Finalize
# =========================================================================

echo ""
echo "================================================================"
echo "  Finalizing rootfs image"
echo "================================================================"

# Set permissions
chmod 01777 "${MOUNT_POINT}/tmp"
chmod 01777 "${MOUNT_POINT}/tmp/.X11-unix"

# Show disk usage
USED_MB=$(du -sm "${MOUNT_POINT}" 2>/dev/null | awk '{print $1}')
echo "  Used space: ${USED_MB:-?} MB / ${ROOTFS_SIZE_MB} MB"

# Unmount handled by trap
sync

echo ""
echo "========================================"
echo "  KDE Root Filesystem Complete"
echo "========================================"
echo ""
echo "  Image:     ${OUTPUT_IMAGE}"
echo "  Size:      ${ROOTFS_SIZE_MB} MB"
echo "  Session:   ${DEFAULT_SESSION}"
echo ""
echo "  To boot with QEMU:"
echo "    ./userland/integration/qemu-kde.sh"
echo ""
echo "  To change default session:"
echo "    Edit /etc/veridian/session.conf in the image"
