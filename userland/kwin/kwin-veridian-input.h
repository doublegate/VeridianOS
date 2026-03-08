/*
 * VeridianOS -- kwin-veridian-input.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libinput-based input backend for KWin on VeridianOS.  Translates
 * libinput events (keyboard, pointer, touch, tablet) into KWin's
 * internal input event types for distribution to Wayland clients.
 *
 * Responsibilities:
 *   - libinput context creation (udev or direct /dev/input/ enumeration)
 *   - Keyboard event translation (libinput -> KWin key events)
 *   - Pointer event translation (motion, button, scroll)
 *   - Touch event translation (down, up, motion, frame)
 *   - Tablet/stylus event stubs (for future expansion)
 *   - Seat management (multi-seat support)
 *   - Device hot-plug monitoring
 */

#ifndef KWIN_VERIDIAN_INPUT_H
#define KWIN_VERIDIAN_INPUT_H

#include <QObject>
#include <QString>
#include <QVector>
#include <QPointF>
#include <QSocketNotifier>
#include <QHash>

/* libinput headers */
#include <libinput.h>

/* xkbcommon for keyboard state */
#include <xkbcommon/xkbcommon.h>

namespace KWin {

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

class VeridianDrmBackend;
class VeridianSession;

/* ========================================================================= */
/* VeridianInputDevice -- per-device state                                   */
/* ========================================================================= */

/**
 * Represents a single input device discovered by libinput.
 *
 * Tracks device capabilities, configuration, and provides a Qt-friendly
 * wrapper around the libinput_device handle.
 */
struct VeridianInputDevice {
    struct libinput_device *device;
    QString name;
    QString sysPath;
    uint32_t vendorId;
    uint32_t productId;

    /* Capability flags */
    bool hasKeyboard;
    bool hasPointer;
    bool hasTouch;
    bool hasTablet;
    bool hasTabletPad;
    bool hasGesture;
    bool hasSwitch;

    /* Configuration */
    bool naturalScroll;
    bool leftHanded;
    bool tapToClick;
    bool tapAndDrag;
    bool middleEmulation;
    double pointerAcceleration;
    enum libinput_config_accel_profile accelProfile;
    enum libinput_config_scroll_method scrollMethod;
    enum libinput_config_click_method clickMethod;

    /* State */
    bool enabled;
};

/* ========================================================================= */
/* VeridianKeyboardState -- xkbcommon keyboard state                         */
/* ========================================================================= */

/**
 * Manages xkbcommon context and keymap state for keyboard event
 * translation.  Supports layout switching and modifier tracking.
 */
class VeridianKeyboardState : public QObject
{
    Q_OBJECT

public:
    explicit VeridianKeyboardState(QObject *parent = nullptr);
    ~VeridianKeyboardState() override;

    bool initialize(const QString &layout = QStringLiteral("us"),
                    const QString &variant = QString(),
                    const QString &model = QStringLiteral("pc105"),
                    const QString &options = QString());

    /* ----- Key processing ----- */
    uint32_t processKey(uint32_t key, bool pressed);
    uint32_t keysym(uint32_t key) const;
    QString keyText(uint32_t key) const;

    /* ----- Modifier state ----- */
    uint32_t modifiers() const;
    bool isShiftPressed() const;
    bool isControlPressed() const;
    bool isAltPressed() const;
    bool isMetaPressed() const;
    uint32_t modifierIndex(const char *name) const;

    /* ----- Layout ----- */
    void switchLayout(uint32_t layoutIndex);
    uint32_t currentLayout() const;
    uint32_t layoutCount() const;
    QString layoutName(uint32_t index) const;

    /* ----- Keymap access (for Wayland seat keyboard) ----- */
    struct xkb_keymap *keymap() const;
    struct xkb_state *xkbState() const;
    int keymapFd() const;
    uint32_t keymapSize() const;

Q_SIGNALS:
    void modifiersChanged(uint32_t depressed, uint32_t latched,
                          uint32_t locked, uint32_t group);
    void layoutChanged(uint32_t layoutIndex);

private:
    bool createKeymapFd();

    struct xkb_context *m_xkbContext;
    struct xkb_keymap *m_xkbKeymap;
    struct xkb_state *m_xkbState;
    int m_keymapFd;
    uint32_t m_keymapSize;
    uint32_t m_currentLayout;

    /* Modifier indices (cached for fast lookup) */
    uint32_t m_modShift;
    uint32_t m_modControl;
    uint32_t m_modAlt;
    uint32_t m_modMeta;
    uint32_t m_modCapsLock;
    uint32_t m_modNumLock;
};

/* ========================================================================= */
/* VeridianInputBackend -- main input backend                                */
/* ========================================================================= */

/**
 * Main input backend for KWin on VeridianOS.
 *
 * Creates a libinput context, monitors /dev/input/ for devices, and
 * translates libinput events into KWin input events.
 *
 * Integration points:
 *   - Uses VeridianSession for device access (logind TakeDevice)
 *   - Uses xkbcommon for keyboard event translation
 *   - Emits Qt signals consumed by KWin's InputRedirection
 *
 * Event flow:
 *   /dev/input/eventN -> libinput -> VeridianInputBackend -> KWin
 */
class VeridianInputBackend : public QObject
{
    Q_OBJECT

public:
    explicit VeridianInputBackend(VeridianSession *session,
                                  QObject *parent = nullptr);
    ~VeridianInputBackend() override;

    /* ----- Initialization ----- */
    bool initialize();
    void suspend();
    void resume();

    /* ----- Device enumeration ----- */
    QVector<VeridianInputDevice> devices() const;
    VeridianInputDevice *deviceByPath(const QString &sysPath);

    /* ----- Device configuration ----- */
    void setPointerAcceleration(const QString &sysPath, double accel);
    void setNaturalScroll(const QString &sysPath, bool enabled);
    void setTapToClick(const QString &sysPath, bool enabled);
    void setLeftHanded(const QString &sysPath, bool enabled);

    /* ----- Keyboard state ----- */
    VeridianKeyboardState *keyboardState() const;
    void setKeyboardLayout(const QString &layout,
                           const QString &variant = QString());

    /* ----- Seat ----- */
    QString seatName() const;

Q_SIGNALS:
    /* Keyboard events */
    void keyPressed(uint32_t key, uint32_t keysym, uint32_t time);
    void keyReleased(uint32_t key, uint32_t keysym, uint32_t time);

    /* Pointer events */
    void pointerMotion(const QPointF &delta, const QPointF &deltaNonAccel,
                       uint32_t time);
    void pointerMotionAbsolute(const QPointF &pos, uint32_t time);
    void pointerButtonPressed(uint32_t button, uint32_t time);
    void pointerButtonReleased(uint32_t button, uint32_t time);
    void pointerAxisVertical(double value, int32_t discreteDelta,
                             uint32_t time);
    void pointerAxisHorizontal(double value, int32_t discreteDelta,
                               uint32_t time);

    /* Touch events */
    void touchDown(int32_t id, const QPointF &pos, uint32_t time);
    void touchUp(int32_t id, uint32_t time);
    void touchMotion(int32_t id, const QPointF &pos, uint32_t time);
    void touchFrame();
    void touchCancel();

    /* Tablet events (stubs for future) */
    void tabletToolEvent(uint32_t type, const QPointF &pos,
                         double pressure, uint32_t time);
    void tabletPadButton(uint32_t button, bool pressed, uint32_t time);

    /* Device events */
    void deviceAdded(const VeridianInputDevice &device);
    void deviceRemoved(const QString &sysPath);

private Q_SLOTS:
    void onLibinputEvent();

private:
    /* ----- libinput interface callbacks ----- */
    static int openRestricted(const char *path, int flags, void *userData);
    static void closeRestricted(int fd, void *userData);

    /* ----- Event dispatch ----- */
    void processEvents();
    void handleKeyboardEvent(struct libinput_event *event);
    void handlePointerMotionEvent(struct libinput_event *event);
    void handlePointerMotionAbsoluteEvent(struct libinput_event *event);
    void handlePointerButtonEvent(struct libinput_event *event);
    void handlePointerAxisEvent(struct libinput_event *event);
    void handleTouchDownEvent(struct libinput_event *event);
    void handleTouchUpEvent(struct libinput_event *event);
    void handleTouchMotionEvent(struct libinput_event *event);
    void handleTouchFrameEvent(struct libinput_event *event);
    void handleTouchCancelEvent(struct libinput_event *event);
    void handleDeviceAddedEvent(struct libinput_event *event);
    void handleDeviceRemovedEvent(struct libinput_event *event);

    /* ----- Device init ----- */
    VeridianInputDevice createDeviceInfo(struct libinput_device *dev);
    void applyDeviceConfig(VeridianInputDevice &info);

    VeridianSession *m_session;
    struct libinput *m_libinput;
    VeridianKeyboardState *m_keyboardState;
    QSocketNotifier *m_notifier;
    QHash<QString, VeridianInputDevice> m_devices;
    QString m_seatName;
};

} /* namespace KWin */

#endif /* KWIN_VERIDIAN_INPUT_H */
