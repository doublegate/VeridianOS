"""at-spi2-core 2.62.0: the three *_keysym_modifier functions of
atspi-device-legacy.c fetch the device's private data only for code under
#ifdef HAVE_X11, so without X11 it is unused (GCC: unused variable 'priv').
Fetching it inside the #ifdef changes nothing for X11 builds.

Usage: python3 atspi_device_legacy_x11_priv.py <at-spi2-core>/atspi/atspi-device-legacy.c
"""

import sys
from pathlib import Path

p = sys.argv[1]
s = Path(p).read_text()
old = """  AtspiDeviceLegacy *legacy_device = ATSPI_DEVICE_LEGACY (device);
  AtspiDeviceLegacyPrivate *priv = atspi_device_legacy_get_instance_private (legacy_device);

  guint resolved_keysym = keysym;
#ifdef HAVE_X11
  if (priv->display)"""
new = """  guint resolved_keysym = keysym;
#ifdef HAVE_X11
  AtspiDeviceLegacy *legacy_device = ATSPI_DEVICE_LEGACY (device);
  AtspiDeviceLegacyPrivate *priv = atspi_device_legacy_get_instance_private (legacy_device);
  if (priv->display)"""
if old not in s and s.count(new) == 3:
    sys.exit(0)  # already patched
if s.count(old) != 3:
    sys.exit("unexpected atspi-device-legacy.c")
Path(p).write_text(s.replace(old, new))
