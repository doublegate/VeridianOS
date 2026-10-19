#!/usr/bin/env python3
"""Write tools/cross/PATCHES.md: every source change the cross pipeline makes.

The table is generated from the patches themselves (each starts with a
one-line summary) and the patch scripts in deps-patches/ (their docstring's
first line), so it cannot drift from them. UPSTREAM lists the changes that
fix a defect in the package -- not only a compiler warning, or a cross-build
or VeridianOS matter -- and so are worth offering upstream; once a release
carries one, its patch is deleted at the next version bump.

Usage: list-patches.py  (from anywhere; writes next to this script)
"""

import ast
from pathlib import Path

HERE = Path(__file__).resolve().parent

# Patch directories: (directory, how its subdirectories are named).
PATCH_DIRS = [
    ("toolchain-patches", "toolchain source tree"),
    ("pkg-patches", "package (deps, X11, D-Bus, Wayland, Qt)"),
    ("kf6-patches", "KDE Frameworks module"),
    ("kwin-patches", "KWin (top level) or a KWin-phase package"),
    ("plasma-patches", "Plasma component"),
]

# Fixes for defects in the package itself, with where each stands upstream.
# PREPARED: prepared for submission (rebased onto the project's default
# branch, tested there), not yet submitted. A fix the project already has is
# marked with its upstream commit; such a patch is dropped at the version
# bump that brings it.
PREPARED = "prepared"
UPSTREAM = {
    "kf6-patches/KDocTools/0001": (
        "meinproc6 exited 0 when it could not write its output",
        PREPARED,
    ),
    "kf6-patches/KCMUtils/0001": (
        "kcmmetadatagenerator exited 0 with an unwritten file",
        PREPARED,
    ),
    "kf6-patches/KWallet/0001": (
        "portal secret silently not written; encrypt() of 0 bytes",
        PREPARED,
    ),
    "kf6-patches/KParts/0001": (
        "carried on after a failed temporary file open",
        PREPARED,
    ),
    "kf6-patches/KCoreAddons/0002": (
        "shared-memory segment unmapped twice (other threads)",
        PREPARED,
    ),
    "kf6-patches/KPty/0002": (
        "logout() cleared one byte of the user and host names",
        PREPARED,
    ),
    "kf6-patches/QCA/0001": (
        "alloc_info uninitialized for sizes of 2 GiB or more",
        PREPARED,
    ),
    "kf6-patches/QCA/0002": (
        "OpenSSL 3 port; keystore keys (crashed); 3 TLS memory errors",
        (
            "the memory-error and keystore-key fixes prepared separately; upstream still "
            "supports OpenSSL 1.1 and LibreSSL, so the port itself is not offered"
        ),
    ),
    "kf6-patches/Solid/0001": (
        "~DeviceManagerPrivate dereferenced a null QPointer (crash)",
        PREPARED,
    ),
    "kf6-patches/Solid/0002": (
        "predicate parser: {...} lists passed NULL (crash)",
        PREPARED,
    ),
    "kf6-patches/Solid/0003": ("predicate parser: string lists reversed", PREPARED),
    "kf6-patches/Solid/0004": (
        "Predicate::toString(): stray quote after a string list",
        PREPARED,
    ),
    "plasma-patches/polkit-qt-1/0001": (
        "ConsoleKit seat signals never connected",
        PREPARED,
    ),
    "plasma-patches/libkscreen/0001": (
        "DPMS: uninitialized mode emitted; fake backend open unchecked",
        PREPARED,
    ),
    "plasma-patches/aurorae/0001": (
        "null dereference when config.ui cannot be opened or parsed",
        PREPARED,
    ),
    "plasma-patches/aurorae/0002": (
        "plasma-apply-aurorae needs /usr/bin/bash",
        PREPARED,
    ),
    "plasma-patches/kde-cli-tools/0001": (
        "kdesu without X11 queried undefined options",
        PREPARED,
    ),
    "plasma-patches/kde-cli-tools/0002": (
        "kmimetypefinder: unreadable stdin reported as empty",
        PREPARED,
    ),
    "plasma-patches/kscreen/0001": (
        "kscreen-console: unopenable files reported wrongly",
        "fixed upstream: 91d3a63a (Plasma 6.8)",
    ),
    "plasma-patches/plasma-workspace/0003": (
        "ksmserver legacy select(): aliased restrict fd_sets (UB)",
        "prepared for Plasma/6.7 (the code is gone from master)",
    ),
    "plasma-patches/plasma-workspace/0004": (
        "icon/cursor/GTK KCMs crash on a non-archive or empty file",
        "fixed upstream (78b7f6a10) except an empty archive; that part prepared",
    ),
    "plasma-patches/plasma-workspace/0005": (
        "region & language KCM rows wrong without LC_ADDRESS (musl)",
        PREPARED,
    ),
    "plasma-patches/ksystemstats/0001": (
        "garbage memory values; Tccd sensors overflow a stack array",
        PREPARED,
    ),
    "plasma-patches/libksysguard/0003": (
        "KiB scaled by 1000 with metric units",
        PREPARED,
    ),
    "plasma-patches/plasma-desktop/0003": (
        "libwacom device leaked; non-Wacom styli mismatched",
        "leak prepared; the styli port is in the maintainer's draft MR !3100",
    ),
    "plasma-patches/plasma-desktop/0006": (
        "tablet list leaked a libwacom device per paint",
        PREPARED,
    ),
    "plasma-patches/kdeplasma-addons/0002": (
        "comic applet: out-of-range read for an identifier without ':'",
        "not prepared yet (present on upstream master)",
    ),
    "plasma-patches/powerdevil/0002": (
        "keyboard brightness isSilent property was invalid",
        PREPARED,
    ),
    "pkg-patches/libX11/0002": (
        "Xrm: pointer used after Xrealloc() may have freed it",
        PREPARED,
    ),
    "pkg-patches/libX11/0003": (
        "error database text used as a printf format (64-bit serial printed as 0)",
        "prepared as a conversion fix (upstream keeps the database formats)",
    ),
    "pkg-patches/libX11/0004": (
        "_XkbReadKeyActions: BadLength instead of BadAlloc",
        PREPARED,
    ),
    "pkg-patches/xcb-util-wm/0002": (
        "WM_HINTS reply overflows the caller; WM_CLASS index -1",
        PREPARED,
    ),
    "pkg-patches/xcb-util-wm/0003": (
        "xcb_ewmh_init_atoms: unchecked allocations",
        PREPARED,
    ),
    "pkg-patches/xcb-util-wm/0004": (
        "WM_CLASS class name one past the reply",
        PREPARED,
    ),
    "pkg-patches/xcb-util-wm/0005": (
        "xcb_ewmh_init_atoms_replies: NULL cookie array",
        PREPARED,
    ),
    "pkg-patches/libxkbfile/0001": (
        "uninitialized TOC type in errors; buffer one byte short",
        PREPARED,
    ),
    "pkg-patches/glib/0001": ("two GObject NULL dereferences on invalid ids", PREPARED),
    "pkg-patches/icu/0002": (
        "collation builder data read before construction (UB)",
        "upstream 45a8a28de6f (ICU 79.1), backported",
    ),
    "pkg-patches/icu/0003": (
        "ctestfw: unbounded copy into a 255-byte buffer",
        "prepared (needs an ICU Jira ticket)",
    ),
    "pkg-patches/dbus/0002": (
        "dbus_message_iter_recurse() on a non-container crashed",
        PREPARED,
    ),
    "pkg-patches/socat/0001": (
        "interface name over-read; abstract socket name off by one",
        PREPARED,
    ),
    "pkg-patches/socat/0003": (
        "ipv6-join-source-group error named the wrong value",
        PREPARED,
    ),
    "pkg-patches/socat/0004": (
        "bind-tempname address length (stack over-read)",
        PREPARED,
    ),
    "pkg-patches/socat/0005": ("ip-add-membership with an interface crashed", PREPARED),
    "pkg-patches/qtbase/0001": (
        "qdbusxml2cpp warned on documented signals only",
        PREPARED,
    ),
}

# Warning fixes also prepared for upstream (they change no behaviour there).
OFFERED_WARNINGS = {
    "plasma-patches/libksysguard/0001": "stale translation domain of the process plugins",
    "plasma-patches/plasma5support/0001": "time engine compiled with two translation domains",
}


# Changes for building on another machine than the one the system runs on.
CROSS = {
    "plasma-patches/plasma-desktop/0001",
    "plasma-patches/plasma-desktop/0004",
    "plasma-patches/plasma-workspace/0001",
    "plasma-patches/plasma-systemmonitor/0002",
    "plasma-patches/kinfocenter/0001",
    "plasma-patches/kwallet-pam/0001",
    "plasma-patches/libkscreen/0002",
}

# Changes that let the package build at all with this stack's versions or
# options.
BUILD = {
    "kwin-patches/0001",
    "kwin-patches/0002",
    "kwin-patches/0003",
    "kwin-patches/0004",
    "plasma-patches/kactivitymanagerd/0001",
}


def summary_of_patch(path):
    first = path.read_text(errors="replace").splitlines()[0]
    return first.removeprefix("VeridianOS: ").strip()


def summary_of_script(path):
    doc = ast.get_docstring(ast.parse(path.read_text()))
    return doc.splitlines()[0].strip() if doc else ""


def key_of(path):
    rel = path.relative_to(HERE)
    return f"{rel.parent}/{rel.name[:4]}"


def main():
    out = [
        "# Source changes made by the cross pipeline",
        "",
        "Generated by `list-patches.py` from the patches and patch scripts; do not edit by hand.",
        "",
        "Every change explains itself at the top of its file. Patches are applied with",
        "`patch --forward` to freshly extracted trees and stop the build when they no longer",
        "apply: after a version bump, refresh or drop each patch of that package (a fix the",
        "new release already contains is deleted). Kinds: **defect** fixes a bug in the",
        "package (listed again under *Upstream candidates*); **warning** removes the cause of",
        "a compiler or build-tool warning without changing behaviour; **cross** concerns",
        "building for VeridianOS from another machine; **build** lets the package build",
        "with this stack's versions or options.",
        "",
    ]
    for directory, naming in PATCH_DIRS:
        root = HERE / directory
        patches = sorted(root.rglob("*.patch"))
        if not patches:
            continue
        out += [
            f"## {directory}/ (by {naming})",
            "",
            "| Patch | Kind | Summary |",
            "|---|---|---|",
        ]
        for p in patches:
            key = key_of(p)
            if key in UPSTREAM:
                kind = "defect"
            elif key in CROSS:
                kind = "cross"
            elif key in BUILD:
                kind = "build"
            else:
                kind = "warning"
            out.append(f"| `{p.relative_to(root)}` | {kind} | {summary_of_patch(p)} |")
        out.append("")
    out += [
        "## deps-patches/ (scripts run on the extracted tree)",
        "",
        "| Script | Summary |",
        "|---|---|",
    ]
    for s in sorted((HERE / "deps-patches").glob("*.py")):
        out.append(f"| `{s.name}` | {summary_of_script(s)} |")
    out += [
        "",
        "## Upstream candidates",
        "",
        "Status: *prepared* means rebased onto the project's default branch and tested",
        "there, not yet submitted.",
        "",
        "| Patch | Defect | Upstream |",
        "|---|---|---|",
    ]
    for key, (what, status) in sorted(UPSTREAM.items()):
        out.append(f"| `{key}` | {what} | {status} |")
    for key, what in sorted(OFFERED_WARNINGS.items()):
        out.append(f"| `{key}` | (warning) {what} | {PREPARED} |")
    out.append("")
    missing = [
        k
        for k in [*UPSTREAM, *OFFERED_WARNINGS]
        if not list((HERE / k).parent.glob(k.rsplit("/", 1)[1] + "-*.patch"))
    ]
    if missing:
        raise SystemExit(f"no patch for: {', '.join(missing)}")
    (HERE / "PATCHES.md").write_text("\n".join(out))


if __name__ == "__main__":
    main()
