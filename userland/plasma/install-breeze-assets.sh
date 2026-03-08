#!/bin/sh
# VeridianOS -- install-breeze-assets.sh
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Installs Breeze theme assets into the VeridianOS sysroot:
#   - Breeze icon theme (index + placeholder categories)
#   - Breeze cursor theme (default cursor set definition)
#   - Breeze color scheme files (light + dark)
#   - Default wallpaper
#   - XDG desktop entries for core KDE applications
#   - Breeze sounds placeholder
#
# Usage:
#   ./install-breeze-assets.sh [sysroot]
#
# The sysroot defaults to /opt/veridian-sysroot.  Assets are installed
# under $SYSROOT/usr/share/.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SYSROOT="${1:-/opt/veridian-sysroot}"

echo "========================================"
echo "  Breeze Asset Installer for VeridianOS"
echo "========================================"
echo "  Sysroot: ${SYSROOT}"
echo "========================================"

# =========================================================================
# Directory structure
# =========================================================================

echo ""
echo "Creating directory structure..."

SHARE="${SYSROOT}/usr/share"
mkdir -p "${SHARE}/icons/breeze/actions/22"
mkdir -p "${SHARE}/icons/breeze/actions/24"
mkdir -p "${SHARE}/icons/breeze/actions/symbolic"
mkdir -p "${SHARE}/icons/breeze/apps/48"
mkdir -p "${SHARE}/icons/breeze/apps/64"
mkdir -p "${SHARE}/icons/breeze/categories/32"
mkdir -p "${SHARE}/icons/breeze/devices/64"
mkdir -p "${SHARE}/icons/breeze/emblems/16"
mkdir -p "${SHARE}/icons/breeze/mimetypes/64"
mkdir -p "${SHARE}/icons/breeze/places/64"
mkdir -p "${SHARE}/icons/breeze/status/22"
mkdir -p "${SHARE}/icons/breeze/status/symbolic"
mkdir -p "${SHARE}/icons/breeze-dark/actions/22"
mkdir -p "${SHARE}/icons/breeze-dark/apps/48"
mkdir -p "${SHARE}/icons/breeze-dark/status/22"
mkdir -p "${SHARE}/icons/breeze_cursors/cursors"
mkdir -p "${SHARE}/color-schemes"
mkdir -p "${SHARE}/wallpapers/VeridianOS/contents/images"
mkdir -p "${SHARE}/applications"
mkdir -p "${SHARE}/sounds/ocean"
mkdir -p "${SHARE}/plasma/desktoptheme/breeze"
mkdir -p "${SHARE}/plasma/look-and-feel/org.kde.breeze.desktop/contents"

echo "  Directories created"

# =========================================================================
# Breeze icon theme index
# =========================================================================

echo ""
echo "Installing Breeze icon theme..."

cat > "${SHARE}/icons/breeze/index.theme" << 'ICONTHEME'
[Icon Theme]
Name=Breeze
Comment=Breeze icon theme for KDE Plasma
Inherits=hicolor
Example=folder

Directories=actions/22,actions/24,actions/symbolic,apps/48,apps/64,categories/32,devices/64,emblems/16,mimetypes/64,places/64,status/22,status/symbolic

[actions/22]
Size=22
Context=Actions
Type=Fixed

[actions/24]
Size=24
Context=Actions
Type=Fixed

[actions/symbolic]
Size=16
Context=Actions
Type=Scalable
MinSize=8
MaxSize=512

[apps/48]
Size=48
Context=Applications
Type=Fixed

[apps/64]
Size=64
Context=Applications
Type=Fixed

[categories/32]
Size=32
Context=Categories
Type=Fixed

[devices/64]
Size=64
Context=Devices
Type=Fixed

[emblems/16]
Size=16
Context=Emblems
Type=Fixed

[mimetypes/64]
Size=64
Context=MimeTypes
Type=Fixed

[places/64]
Size=64
Context=Places
Type=Fixed

[status/22]
Size=22
Context=Status
Type=Fixed

[status/symbolic]
Size=16
Context=Status
Type=Scalable
MinSize=8
MaxSize=512
ICONTHEME

# Breeze Dark icon theme index
cat > "${SHARE}/icons/breeze-dark/index.theme" << 'DARKTHEME'
[Icon Theme]
Name=Breeze Dark
Comment=Breeze Dark icon theme for KDE Plasma
Inherits=breeze,hicolor
Example=folder

Directories=actions/22,apps/48,status/22

[actions/22]
Size=22
Context=Actions
Type=Fixed

[apps/48]
Size=48
Context=Applications
Type=Fixed

[status/22]
Size=22
Context=Status
Type=Fixed
DARKTHEME

echo "  Icon theme indices installed"

# =========================================================================
# Breeze cursor theme
# =========================================================================

echo ""
echo "Installing Breeze cursor theme..."

cat > "${SHARE}/icons/breeze_cursors/index.theme" << 'CURSORTHEME'
[Icon Theme]
Name=Breeze
Comment=Breeze cursor theme
Inherits=default
CURSORTHEME

# Cursor theme metadata
cat > "${SHARE}/icons/breeze_cursors/cursor.theme" << 'CURSORINFO'
[Icon Theme]
Name=Breeze
Comment=Breeze cursor theme for VeridianOS
Inherits=default

# Cursors are installed from the upstream Breeze cursor package.
# This file provides the theme metadata for cursor theme discovery.
# The actual cursor images (X11 xcursor format) would be placed in
# the cursors/ subdirectory during the full KDE build.
#
# Standard cursor names expected:
#   left_ptr, text, pointer, wait, help, crosshair, move,
#   top_left_corner, top_right_corner, bottom_left_corner,
#   bottom_right_corner, top_side, bottom_side, left_side,
#   right_side, sb_h_double_arrow, sb_v_double_arrow
CURSORINFO

echo "  Cursor theme installed"

# =========================================================================
# Color schemes
# =========================================================================

echo ""
echo "Installing Breeze color schemes..."

cat > "${SHARE}/color-schemes/BreezeLight.colors" << 'LIGHTSCHEME'
[ColorEffects:Disabled]
Color=56,56,56
ColorAmount=0
ColorEffect=0
ContrastAmount=0.65
ContrastEffect=1
IntensityAmount=0.1
IntensityEffect=2

[ColorEffects:Inactive]
ChangeSelectionColor=true
Color=112,111,110
ColorAmount=0.025
ColorEffect=2
ContrastAmount=0.1
ContrastEffect=2
Enable=false
IntensityAmount=0
IntensityEffect=0

[Colors:Button]
BackgroundAlternate=189,195,199
BackgroundNormal=239,240,241
DecorationFocus=61,174,233
DecorationHover=147,206,233
ForegroundActive=61,174,233
ForegroundInactive=127,140,141
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=35,38,41
ForegroundPositive=39,174,96
ForegroundVisited=127,140,141

[Colors:Selection]
BackgroundAlternate=29,153,243
BackgroundNormal=61,174,233
DecorationFocus=61,174,233
DecorationHover=147,206,233
ForegroundActive=252,252,252
ForegroundInactive=252,252,252
ForegroundLink=253,188,75
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=252,252,252
ForegroundPositive=39,174,96
ForegroundVisited=189,195,199

[Colors:Tooltip]
BackgroundAlternate=239,240,241
BackgroundNormal=247,247,247
DecorationFocus=61,174,233
DecorationHover=147,206,233
ForegroundActive=61,174,233
ForegroundInactive=127,140,141
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=35,38,41
ForegroundPositive=39,174,96
ForegroundVisited=127,140,141

[Colors:View]
BackgroundAlternate=239,240,241
BackgroundNormal=252,252,252
DecorationFocus=61,174,233
DecorationHover=147,206,233
ForegroundActive=61,174,233
ForegroundInactive=127,140,141
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=35,38,41
ForegroundPositive=39,174,96
ForegroundVisited=127,140,141

[Colors:Window]
BackgroundAlternate=227,229,231
BackgroundNormal=239,240,241
DecorationFocus=61,174,233
DecorationHover=147,206,233
ForegroundActive=61,174,233
ForegroundInactive=127,140,141
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=35,38,41
ForegroundPositive=39,174,96
ForegroundVisited=127,140,141

[General]
ColorScheme=BreezeLight
Name=Breeze Light
shadeSortColumn=true

[WM]
activeBackground=227,229,231
activeBlend=227,229,231
activeForeground=35,38,41
inactiveBackground=239,240,241
inactiveBlend=239,240,241
inactiveForeground=127,140,141
LIGHTSCHEME

cat > "${SHARE}/color-schemes/BreezeDark.colors" << 'DARKSCHEME'
[ColorEffects:Disabled]
Color=56,56,56
ColorAmount=0
ColorEffect=0
ContrastAmount=0.65
ContrastEffect=1
IntensityAmount=0.1
IntensityEffect=2

[ColorEffects:Inactive]
ChangeSelectionColor=true
Color=112,111,110
ColorAmount=0.025
ColorEffect=2
ContrastAmount=0.1
ContrastEffect=2
Enable=false
IntensityAmount=0
IntensityEffect=0

[Colors:Button]
BackgroundAlternate=42,46,50
BackgroundNormal=49,54,59
DecorationFocus=61,174,233
DecorationHover=61,174,233
ForegroundActive=61,174,233
ForegroundInactive=161,169,177
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=239,240,241
ForegroundPositive=39,174,96
ForegroundVisited=155,89,182

[Colors:Selection]
BackgroundAlternate=29,153,243
BackgroundNormal=61,174,233
DecorationFocus=61,174,233
DecorationHover=61,174,233
ForegroundActive=252,252,252
ForegroundInactive=252,252,252
ForegroundLink=253,188,75
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=252,252,252
ForegroundPositive=39,174,96
ForegroundVisited=189,195,199

[Colors:Tooltip]
BackgroundAlternate=42,46,50
BackgroundNormal=49,54,59
DecorationFocus=61,174,233
DecorationHover=61,174,233
ForegroundActive=61,174,233
ForegroundInactive=161,169,177
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=239,240,241
ForegroundPositive=39,174,96
ForegroundVisited=155,89,182

[Colors:View]
BackgroundAlternate=42,46,50
BackgroundNormal=35,38,41
DecorationFocus=61,174,233
DecorationHover=61,174,233
ForegroundActive=61,174,233
ForegroundInactive=161,169,177
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=239,240,241
ForegroundPositive=39,174,96
ForegroundVisited=155,89,182

[Colors:Window]
BackgroundAlternate=42,46,50
BackgroundNormal=49,54,59
DecorationFocus=61,174,233
DecorationHover=61,174,233
ForegroundActive=61,174,233
ForegroundInactive=161,169,177
ForegroundLink=41,128,185
ForegroundNegative=218,68,83
ForegroundNeutral=246,116,0
ForegroundNormal=239,240,241
ForegroundPositive=39,174,96
ForegroundVisited=155,89,182

[General]
ColorScheme=BreezeDark
Name=Breeze Dark
shadeSortColumn=true

[WM]
activeBackground=49,54,59
activeBlend=49,54,59
activeForeground=239,240,241
inactiveBackground=42,46,50
inactiveBlend=42,46,50
inactiveForeground=127,140,141
DARKSCHEME

echo "  Color schemes installed"

# =========================================================================
# Default wallpaper
# =========================================================================

echo ""
echo "Installing default wallpaper..."

cat > "${SHARE}/wallpapers/VeridianOS/metadata.json" << 'WALLMETA'
{
    "KPlugin": {
        "Id": "VeridianOS",
        "Name": "VeridianOS Default",
        "Description": "Default wallpaper for VeridianOS with KDE Plasma",
        "Authors": [
            {
                "Name": "VeridianOS Contributors"
            }
        ],
        "License": "CC-BY-SA-4.0"
    }
}
WALLMETA

# Create a simple gradient wallpaper placeholder (PPM format, 16x9 thumbnail).
# The actual wallpaper image (PNG/JPG) would be installed from the art package.
cat > "${SHARE}/wallpapers/VeridianOS/contents/images/README" << 'WALLREADME'
VeridianOS Default Wallpaper
============================

Place wallpaper images here in the following sizes:
  - 1920x1080.png  (Full HD)
  - 2560x1440.png  (QHD)
  - 3840x2160.png  (4K UHD)

The default VeridianOS wallpaper uses a dark blue-to-teal gradient
with the VeridianOS logo centered.

Color palette:
  Top:    #1a1a2e (Dark navy)
  Middle: #16213e (Deep blue)
  Bottom: #0f3460 (Ocean blue)
  Accent: #3daee9 (Breeze blue)
WALLREADME

echo "  Wallpaper placeholder installed"

# =========================================================================
# Plasma desktop theme
# =========================================================================

echo ""
echo "Installing Plasma desktop theme metadata..."

cat > "${SHARE}/plasma/desktoptheme/breeze/metadata.json" << 'PLASMATHEME'
{
    "KPlugin": {
        "Id": "breeze",
        "Name": "Breeze",
        "Description": "Breeze Plasma theme",
        "Authors": [
            {
                "Name": "KDE Visual Design Group"
            }
        ],
        "License": "LGPL-2.1-or-later",
        "Website": "https://kde.org"
    },
    "X-Plasma-API": "5.0"
}
PLASMATHEME

echo "  Desktop theme metadata installed"

# =========================================================================
# Look and Feel package
# =========================================================================

echo ""
echo "Installing Breeze Look and Feel package..."

cat > "${SHARE}/plasma/look-and-feel/org.kde.breeze.desktop/metadata.json" << 'LNFMETA'
{
    "KPlugin": {
        "Id": "org.kde.breeze.desktop",
        "Name": "Breeze",
        "Description": "Breeze Look and Feel for KDE Plasma on VeridianOS",
        "Authors": [
            {
                "Name": "KDE Visual Design Group"
            }
        ],
        "License": "LGPL-2.1-or-later"
    },
    "X-Plasma-MainScript": "defaults",
    "KPackageStructure": "Plasma/LookAndFeel"
}
LNFMETA

cat > "${SHARE}/plasma/look-and-feel/org.kde.breeze.desktop/contents/defaults" << 'LNFDEFAULTS'
[kdeglobals][General]
ColorScheme=BreezeLight
widgetStyle=breeze

[kdeglobals][Icons]
Theme=breeze

[kdeglobals][KDE]
LookAndFeelPackage=org.kde.breeze.desktop

[kwinrc][org.kde.kdecoration2]
library=org.kde.breeze
theme=Breeze

[plasmarc][Theme]
name=breeze

[kcminputrc][Mouse]
cursorTheme=breeze_cursors
cursorSize=24

[kscreenlockerrc][Greeter]
WallpaperPlugin=org.kde.image
Theme=org.kde.breeze.desktop
LNFDEFAULTS

echo "  Look and Feel package installed"

# =========================================================================
# Desktop entry files for core KDE apps
# =========================================================================

echo ""
echo "Installing .desktop files..."

# Copy from applications/ subdirectory if available
if [ -d "${SCRIPT_DIR}/applications" ]; then
    for DESKTOP_FILE in "${SCRIPT_DIR}"/applications/*.desktop; do
        if [ -f "${DESKTOP_FILE}" ]; then
            cp -f "${DESKTOP_FILE}" "${SHARE}/applications/"
            echo "  Installed $(basename "${DESKTOP_FILE}")"
        fi
    done
else
    echo "  WARNING: applications/ directory not found, skipping .desktop files"
fi

# =========================================================================
# Sound theme placeholder
# =========================================================================

echo ""
echo "Installing sound theme placeholder..."

cat > "${SHARE}/sounds/ocean/index.theme" << 'SOUNDTHEME'
[Sound Theme]
Name=Ocean
Comment=KDE Plasma default sound theme
Directories=stereo

[stereo]
OutputProfile=stereo
SOUNDTHEME

echo "  Sound theme placeholder installed"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "========================================"
echo "  Breeze Asset Installation Complete"
echo "========================================"
echo ""
echo "  Installed to: ${SHARE}"
echo ""
echo "  Components:"
echo "    - Icon theme:     ${SHARE}/icons/breeze/"
echo "    - Icon theme:     ${SHARE}/icons/breeze-dark/"
echo "    - Cursor theme:   ${SHARE}/icons/breeze_cursors/"
echo "    - Color schemes:  ${SHARE}/color-schemes/"
echo "    - Wallpaper:      ${SHARE}/wallpapers/VeridianOS/"
echo "    - Desktop theme:  ${SHARE}/plasma/desktoptheme/breeze/"
echo "    - Look and Feel:  ${SHARE}/plasma/look-and-feel/"
echo "    - Desktop files:  ${SHARE}/applications/"
echo "    - Sound theme:    ${SHARE}/sounds/ocean/"
echo ""
echo "  Note: Icon SVGs and cursor images are installed from the"
echo "  upstream Breeze packages during the full KDE build (build-plasma-apps.sh)."
echo "  This script installs the theme metadata and configuration."
