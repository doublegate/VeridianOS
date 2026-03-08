/*
 * VeridianOS -- kwin-veridian-session.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin session startup and management for VeridianOS.
 *
 * Handles the complete lifecycle of a KWin compositor session:
 *   1. XDG_RUNTIME_DIR setup and validation
 *   2. D-Bus session bus connection verification
 *   3. logind session activation via systemd-logind D-Bus API
 *   4. KWin process launch with correct environment variables
 *   5. Wayland socket creation verification
 *   6. Signal handling for clean shutdown (SIGTERM, SIGINT)
 *
 * This is the entry point for starting KWin under VeridianOS.
 * It is invoked by the session startup script after D-Bus is running.
 *
 * Environment requirements:
 *   XDG_RUNTIME_DIR  - /run/user/<uid> (created by logind or login shell)
 *   DBUS_SESSION_BUS_ADDRESS - Set by dbus-launch or dbus-daemon
 *   WAYLAND_DISPLAY   - Set by this launcher (default: "wayland-0")
 */

#include "kwin-veridian-platform.h"
#include "kwin-veridian-input.h"
#include "kwin-veridian-protocols.h"

#include <QCoreApplication>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QProcess>
#include <QSocketNotifier>
#include <QString>
#include <QStringList>
#include <QTimer>

#include <unistd.h>
#include <signal.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <errno.h>
#include <string.h>
#include <pwd.h>

namespace KWin {

/* ========================================================================= */
/* Signal handling                                                           */
/* ========================================================================= */

static int s_signalPipeFd[2] = { -1, -1 };

/**
 * POSIX signal handler -- writes signal number to pipe for safe
 * processing in the Qt event loop (async-signal-safe).
 */
static void signalHandler(int signum)
{
    char sig = static_cast<char>(signum);
    if (s_signalPipeFd[1] >= 0) {
        /* write() is async-signal-safe */
        ssize_t ret = write(s_signalPipeFd[1], &sig, 1);
        (void)ret; /* Ignore write errors in signal handler */
    }
}

/**
 * Install signal handlers for clean shutdown.
 *
 * Uses a self-pipe trick to safely handle POSIX signals in the Qt
 * event loop without calling non-async-signal-safe functions.
 */
static bool installSignalHandlers(QCoreApplication *app)
{
    if (pipe(s_signalPipeFd) != 0) {
        qWarning("KWinSession: pipe() for signal handling failed: %s",
                 strerror(errno));
        return false;
    }

    /* Monitor the read end of the pipe in the Qt event loop */
    auto *notifier = new QSocketNotifier(s_signalPipeFd[0],
                                          QSocketNotifier::Read, app);
    QObject::connect(notifier, &QSocketNotifier::activated, app, [app]() {
        char sig;
        ssize_t ret = read(s_signalPipeFd[0], &sig, 1);
        if (ret > 0) {
            qDebug("KWinSession: received signal %d, initiating shutdown",
                   static_cast<int>(sig));
            app->quit();
        }
    });

    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = signalHandler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_RESTART;

    sigaction(SIGTERM, &sa, nullptr);
    sigaction(SIGINT, &sa, nullptr);
    sigaction(SIGHUP, &sa, nullptr);

    return true;
}

/* ========================================================================= */
/* XDG runtime directory                                                     */
/* ========================================================================= */

/**
 * Ensure XDG_RUNTIME_DIR exists with correct permissions.
 *
 * On VeridianOS, logind creates /run/user/<uid>/ during login.
 * If it doesn't exist (e.g., when testing without logind), we create
 * it ourselves with mode 0700.
 */
static QString ensureXdgRuntimeDir()
{
    QString runtimeDir = qEnvironmentVariable("XDG_RUNTIME_DIR");

    if (runtimeDir.isEmpty()) {
        /* Construct from UID */
        uid_t uid = getuid();
        runtimeDir = QStringLiteral("/run/user/%1").arg(uid);

        /* Try /tmp fallback if /run/user doesn't exist */
        QDir runUser(QStringLiteral("/run/user"));
        if (!runUser.exists()) {
            runtimeDir = QStringLiteral("/tmp/veridian-runtime-%1").arg(uid);
        }
    }

    QDir dir(runtimeDir);
    if (!dir.exists()) {
        if (!dir.mkpath(QStringLiteral("."))) {
            qWarning("KWinSession: failed to create XDG_RUNTIME_DIR: %s",
                     qPrintable(runtimeDir));
            return QString();
        }

        /* Set permissions to 0700 (owner only) */
        chmod(runtimeDir.toUtf8().constData(), 0700);
    }

    /* Verify ownership and permissions */
    struct stat st;
    if (stat(runtimeDir.toUtf8().constData(), &st) == 0) {
        if (st.st_uid != getuid()) {
            qWarning("KWinSession: XDG_RUNTIME_DIR owned by wrong user");
            return QString();
        }
        if ((st.st_mode & 0777) != 0700) {
            qWarning("KWinSession: XDG_RUNTIME_DIR has wrong permissions "
                     "(0%o, expected 0700)", st.st_mode & 0777);
            /* Fix permissions */
            chmod(runtimeDir.toUtf8().constData(), 0700);
        }
    }

    /* Export to environment */
    qputenv("XDG_RUNTIME_DIR", runtimeDir.toUtf8());

    qDebug("KWinSession: XDG_RUNTIME_DIR = %s", qPrintable(runtimeDir));
    return runtimeDir;
}

/* ========================================================================= */
/* XDG base directories                                                      */
/* ========================================================================= */

/**
 * Set up XDG base directories for KDE configuration and data.
 */
static void setupXdgDirectories()
{
    QString homeDir = QDir::homePath();

    /* XDG_CONFIG_HOME (default: ~/.config) */
    if (qEnvironmentVariable("XDG_CONFIG_HOME").isEmpty()) {
        QString configHome = homeDir + QStringLiteral("/.config");
        qputenv("XDG_CONFIG_HOME", configHome.toUtf8());
        QDir().mkpath(configHome);
    }

    /* XDG_DATA_HOME (default: ~/.local/share) */
    if (qEnvironmentVariable("XDG_DATA_HOME").isEmpty()) {
        QString dataHome = homeDir + QStringLiteral("/.local/share");
        qputenv("XDG_DATA_HOME", dataHome.toUtf8());
        QDir().mkpath(dataHome);
    }

    /* XDG_CACHE_HOME (default: ~/.cache) */
    if (qEnvironmentVariable("XDG_CACHE_HOME").isEmpty()) {
        QString cacheHome = homeDir + QStringLiteral("/.cache");
        qputenv("XDG_CACHE_HOME", cacheHome.toUtf8());
        QDir().mkpath(cacheHome);
    }

    /* XDG_DATA_DIRS -- include system KDE data */
    QString dataDirs = qEnvironmentVariable("XDG_DATA_DIRS");
    if (dataDirs.isEmpty()) {
        dataDirs = QStringLiteral("/usr/local/share:/usr/share");
    }
    qputenv("XDG_DATA_DIRS", dataDirs.toUtf8());

    /* XDG_CONFIG_DIRS */
    QString configDirs = qEnvironmentVariable("XDG_CONFIG_DIRS");
    if (configDirs.isEmpty()) {
        configDirs = QStringLiteral("/etc/xdg");
    }
    qputenv("XDG_CONFIG_DIRS", configDirs.toUtf8());
}

/* ========================================================================= */
/* D-Bus session bus                                                         */
/* ========================================================================= */

/**
 * Verify or start the D-Bus session bus.
 *
 * KWin and all KDE components require a session bus for inter-process
 * communication.  On VeridianOS, dbus-daemon is started by the init
 * system (Sprint 9.5).
 */
static bool ensureDbusSession()
{
    QString busAddress = qEnvironmentVariable("DBUS_SESSION_BUS_ADDRESS");

    if (!busAddress.isEmpty()) {
        qDebug("KWinSession: D-Bus session bus at %s", qPrintable(busAddress));
        return true;
    }

    /* Try to start dbus-launch */
    qDebug("KWinSession: DBUS_SESSION_BUS_ADDRESS not set, attempting dbus-launch");

    QProcess dbus;
    dbus.setProgram(QStringLiteral("dbus-launch"));
    dbus.setArguments({QStringLiteral("--sh-syntax")});
    dbus.start();

    if (!dbus.waitForFinished(5000)) {
        qWarning("KWinSession: dbus-launch failed or timed out");
        return false;
    }

    /* Parse dbus-launch output:
     *   DBUS_SESSION_BUS_ADDRESS='unix:abstract=/tmp/dbus-XXXXX,...';
     *   DBUS_SESSION_BUS_PID=12345; */
    QString output = QString::fromUtf8(dbus.readAllStandardOutput());
    QStringList lines = output.split('\n');

    for (const QString &line : lines) {
        if (line.startsWith(QStringLiteral("DBUS_SESSION_BUS_ADDRESS="))) {
            QString addr = line.mid(25);
            addr.remove('\'');
            addr.remove(';');
            qputenv("DBUS_SESSION_BUS_ADDRESS", addr.toUtf8());
            qDebug("KWinSession: started D-Bus at %s", qPrintable(addr));
        } else if (line.startsWith(QStringLiteral("DBUS_SESSION_BUS_PID="))) {
            QString pid = line.mid(21);
            pid.remove(';');
            qputenv("DBUS_SESSION_BUS_PID", pid.toUtf8());
        }
    }

    return !qEnvironmentVariable("DBUS_SESSION_BUS_ADDRESS").isEmpty();
}

/* ========================================================================= */
/* Wayland socket verification                                               */
/* ========================================================================= */

/**
 * Check that KWin created the Wayland socket.
 *
 * After KWin starts, it creates $XDG_RUNTIME_DIR/wayland-0 (or the
 * name specified in $WAYLAND_DISPLAY).  Clients connect to this socket.
 */
static bool verifyWaylandSocket(const QString &runtimeDir,
                                 const QString &displayName)
{
    QString socketPath = runtimeDir + QStringLiteral("/") + displayName;

    if (QFile::exists(socketPath)) {
        qDebug("KWinSession: Wayland socket verified: %s",
               qPrintable(socketPath));
        return true;
    }

    /* Also check for the lock file (wayland-0.lock) */
    QString lockPath = socketPath + QStringLiteral(".lock");
    if (QFile::exists(lockPath)) {
        qDebug("KWinSession: Wayland lock file exists: %s",
               qPrintable(lockPath));
        return true;
    }

    qWarning("KWinSession: Wayland socket not found at %s",
             qPrintable(socketPath));
    return false;
}

/* ========================================================================= */
/* KWin session entry point                                                  */
/* ========================================================================= */

/**
 * Start a KWin compositor session on VeridianOS.
 *
 * This function is the main entry point for the KWin session launcher.
 * It sets up the environment, initializes the DRM backend, input
 * backend, and KDE Wayland protocols, then enters the Qt event loop.
 *
 * @param argc  Command-line argument count
 * @param argv  Command-line arguments
 * @return 0 on clean shutdown, non-zero on error
 *
 * Supported arguments:
 *   --wayland-display <name>  Wayland socket name (default: wayland-0)
 *   --drm-device <path>       DRM device path (default: /dev/dri/card0)
 *   --layout <name>           Keyboard layout (default: us)
 */
int startKwinSession(int argc, char *argv[])
{
    QCoreApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("kwin_wayland"));
    app.setApplicationVersion(QStringLiteral("6.2.0"));

    qDebug("KWinSession: starting KWin compositor on VeridianOS");

    /* Parse command-line arguments */
    QString waylandDisplay = QStringLiteral("wayland-0");
    QString drmDevice = QStringLiteral("/dev/dri/card0");
    QString kbLayout = QStringLiteral("us");
    QString kbVariant;

    QStringList args = app.arguments();
    for (int i = 1; i < args.size(); ++i) {
        if (args[i] == QStringLiteral("--wayland-display") && i + 1 < args.size()) {
            waylandDisplay = args[++i];
        } else if (args[i] == QStringLiteral("--drm-device") && i + 1 < args.size()) {
            drmDevice = args[++i];
        } else if (args[i] == QStringLiteral("--layout") && i + 1 < args.size()) {
            kbLayout = args[++i];
        } else if (args[i] == QStringLiteral("--variant") && i + 1 < args.size()) {
            kbVariant = args[++i];
        }
    }

    /* Step 1: Install signal handlers */
    installSignalHandlers(&app);

    /* Step 2: Set up XDG directories */
    QString runtimeDir = ensureXdgRuntimeDir();
    if (runtimeDir.isEmpty()) {
        qCritical("KWinSession: cannot set up XDG_RUNTIME_DIR");
        return 1;
    }
    setupXdgDirectories();

    /* Step 3: Verify D-Bus session bus */
    if (!ensureDbusSession()) {
        qCritical("KWinSession: D-Bus session bus not available");
        return 1;
    }

    /* Step 4: Set WAYLAND_DISPLAY for clients */
    qputenv("WAYLAND_DISPLAY", waylandDisplay.toUtf8());

    /* Step 5: Set Qt platform to Wayland (for KDE apps launched under KWin) */
    qputenv("QT_QPA_PLATFORM", "wayland");

    /* Step 6: Initialize DRM backend */
    qDebug("KWinSession: initializing DRM backend on %s", qPrintable(drmDevice));

    VeridianDrmBackend drmBackend;
    if (!drmBackend.openDrmDevice(drmDevice)) {
        qCritical("KWinSession: DRM backend initialization failed");
        return 1;
    }

    if (!drmBackend.initGbm()) {
        qCritical("KWinSession: GBM initialization failed");
        return 1;
    }

    if (!drmBackend.initEglBackend()) {
        qCritical("KWinSession: EGL initialization failed");
        return 1;
    }

    if (!drmBackend.scanConnectors()) {
        qCritical("KWinSession: no display outputs found");
        return 1;
    }

    qDebug("KWinSession: %d output(s) configured", drmBackend.outputs().size());

    /* Step 7: Initialize input backend */
    qDebug("KWinSession: initializing input backend");

    VeridianInputBackend inputBackend(drmBackend.session());
    if (!inputBackend.initialize()) {
        qWarning("KWinSession: input backend initialization failed (non-fatal)");
    }

    /* Configure keyboard layout */
    inputBackend.setKeyboardLayout(kbLayout, kbVariant);

    /* Step 8: Configure effects based on GPU capabilities */
    qDebug("KWinSession: configuring effects");
    bool configuredEffects = configureVeridianEffects(drmBackend.eglBackend());
    if (!configuredEffects) {
        qWarning("KWinSession: effect configuration failed (non-fatal)");
    }

    /* Step 9: Verify Wayland socket (will be created by KWin's Wayland server) */
    /* Use a delayed check since the socket is created asynchronously */
    QTimer::singleShot(2000, &app, [&runtimeDir, &waylandDisplay]() {
        if (!verifyWaylandSocket(runtimeDir, waylandDisplay)) {
            qWarning("KWinSession: Wayland socket not yet available "
                     "(may still be starting)");
        }
    });

    /* Step 10: Log startup summary */
    qDebug("KWinSession: startup complete");
    qDebug("  Wayland display: %s", qPrintable(waylandDisplay));
    qDebug("  DRM device:      %s", qPrintable(drmDevice));
    qDebug("  DRM driver:      %s", qPrintable(drmBackend.driverName()));
    qDebug("  Outputs:         %d", drmBackend.outputs().size());
    if (drmBackend.primaryOutput()) {
        QSize size = drmBackend.primaryOutput()->sizePixels();
        int refresh = drmBackend.primaryOutput()->refreshRate();
        qDebug("  Primary:         %dx%d @ %d.%03d Hz",
               size.width(), size.height(), refresh / 1000, refresh % 1000);
    }
    qDebug("  GL renderer:     %s",
           qPrintable(drmBackend.eglBackend()->glRenderer()));
    qDebug("  llvmpipe:        %s",
           drmBackend.eglBackend()->isLlvmpipe() ? "yes" : "no");
    qDebug("  Input devices:   %d", inputBackend.devices().size());
    qDebug("  Keyboard layout: %s%s%s",
           qPrintable(kbLayout),
           kbVariant.isEmpty() ? "" : " (",
           kbVariant.isEmpty() ? "" : qPrintable(kbVariant + QStringLiteral(")")));

    /* Enter event loop */
    int ret = app.exec();

    /* Clean shutdown */
    qDebug("KWinSession: shutting down (exit code %d)", ret);

    /* Close signal pipe */
    if (s_signalPipeFd[0] >= 0)
        close(s_signalPipeFd[0]);
    if (s_signalPipeFd[1] >= 0)
        close(s_signalPipeFd[1]);

    return ret;
}

/* Forward declaration for effects configuration (defined in kwin-veridian-effects.cpp) */
extern bool configureVeridianEffects(VeridianEglBackend *eglBackend);

} /* namespace KWin */
