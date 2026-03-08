/*
 * VeridianOS -- kwin-veridian-input.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libinput-based input backend implementation for KWin on VeridianOS.
 *
 * Event flow:
 *   /dev/input/eventN -> libinput -> VeridianInputBackend -> Qt signals
 *                                                          -> KWin InputRedirection
 *
 * Device access is managed via VeridianSession (logind TakeDevice/
 * ReleaseDevice) to ensure proper permissions without running as root.
 */

#include "kwin-veridian-input.h"
#include "kwin-veridian-platform.h"

#include <QDebug>
#include <QDir>
#include <QTemporaryFile>

#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <string.h>
#include <sys/mman.h>
#include <linux/input-event-codes.h>

namespace KWin {

/* ========================================================================= */
/* VeridianKeyboardState                                                     */
/* ========================================================================= */

VeridianKeyboardState::VeridianKeyboardState(QObject *parent)
    : QObject(parent)
    , m_xkbContext(nullptr)
    , m_xkbKeymap(nullptr)
    , m_xkbState(nullptr)
    , m_keymapFd(-1)
    , m_keymapSize(0)
    , m_currentLayout(0)
    , m_modShift(0)
    , m_modControl(0)
    , m_modAlt(0)
    , m_modMeta(0)
    , m_modCapsLock(0)
    , m_modNumLock(0)
{
}

VeridianKeyboardState::~VeridianKeyboardState()
{
    if (m_keymapFd >= 0)
        ::close(m_keymapFd);
    if (m_xkbState)
        xkb_state_unref(m_xkbState);
    if (m_xkbKeymap)
        xkb_keymap_unref(m_xkbKeymap);
    if (m_xkbContext)
        xkb_context_unref(m_xkbContext);
}

bool VeridianKeyboardState::initialize(const QString &layout,
                                       const QString &variant,
                                       const QString &model,
                                       const QString &options)
{
    m_xkbContext = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    if (!m_xkbContext) {
        qWarning("VeridianKeyboardState: xkb_context_new failed");
        return false;
    }

    /* Create keymap from RMLVO names */
    struct xkb_rule_names names;
    memset(&names, 0, sizeof(names));
    names.rules = "evdev";
    names.model = model.toUtf8().constData();
    names.layout = layout.toUtf8().constData();
    names.variant = variant.isEmpty() ? nullptr : variant.toUtf8().constData();
    names.options = options.isEmpty() ? nullptr : options.toUtf8().constData();

    m_xkbKeymap = xkb_keymap_new_from_names(m_xkbContext, &names,
                                             XKB_KEYMAP_COMPILE_NO_FLAGS);
    if (!m_xkbKeymap) {
        qWarning("VeridianKeyboardState: xkb_keymap_new_from_names failed");
        return false;
    }

    m_xkbState = xkb_state_new(m_xkbKeymap);
    if (!m_xkbState) {
        qWarning("VeridianKeyboardState: xkb_state_new failed");
        return false;
    }

    /* Cache modifier indices for fast lookup */
    m_modShift = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_SHIFT);
    m_modControl = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_CTRL);
    m_modAlt = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_ALT);
    m_modMeta = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_LOGO);
    m_modCapsLock = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_CAPS);
    m_modNumLock = xkb_keymap_mod_get_index(m_xkbKeymap, XKB_MOD_NAME_NUM);

    /* Create shared memory fd for keymap (Wayland keyboard protocol) */
    if (!createKeymapFd())
        qWarning("VeridianKeyboardState: keymap fd creation failed (non-fatal)");

    qDebug("VeridianKeyboardState: initialized layout=%s variant=%s model=%s",
           qPrintable(layout),
           variant.isEmpty() ? "(none)" : qPrintable(variant),
           qPrintable(model));

    return true;
}

bool VeridianKeyboardState::createKeymapFd()
{
    /* Serialize keymap to string and write to a shared-memory fd.
     * Wayland clients receive this fd to build their own xkb_state. */
    char *keymapStr = xkb_keymap_get_as_string(m_xkbKeymap,
                                                XKB_KEYMAP_FORMAT_TEXT_V1);
    if (!keymapStr)
        return false;

    m_keymapSize = strlen(keymapStr) + 1;

    /* Create anonymous shared memory */
    m_keymapFd = memfd_create("kwin-keymap", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (m_keymapFd < 0) {
        /* Fallback: use /tmp file */
        char tmpl[] = "/tmp/kwin-keymap-XXXXXX";
        m_keymapFd = mkstemp(tmpl);
        if (m_keymapFd < 0) {
            free(keymapStr);
            return false;
        }
        unlink(tmpl);
    }

    if (ftruncate(m_keymapFd, m_keymapSize) != 0) {
        ::close(m_keymapFd);
        m_keymapFd = -1;
        free(keymapStr);
        return false;
    }

    void *ptr = mmap(nullptr, m_keymapSize, PROT_WRITE, MAP_SHARED, m_keymapFd, 0);
    if (ptr == MAP_FAILED) {
        ::close(m_keymapFd);
        m_keymapFd = -1;
        free(keymapStr);
        return false;
    }

    memcpy(ptr, keymapStr, m_keymapSize);
    munmap(ptr, m_keymapSize);
    free(keymapStr);

    return true;
}

uint32_t VeridianKeyboardState::processKey(uint32_t key, bool pressed)
{
    /* libinput uses Linux input event codes (evdev).  xkbcommon expects
     * evdev keycode + 8 for the XKB offset. */
    xkb_keycode_t xkbKey = key + 8;
    enum xkb_key_direction dir = pressed ? XKB_KEY_DOWN : XKB_KEY_UP;

    xkb_state_update_key(m_xkbState, xkbKey, dir);

    /* Get keysym for the key */
    xkb_keysym_t sym = xkb_state_key_get_one_sym(m_xkbState, xkbKey);

    /* Emit modifier change if modifiers were affected */
    uint32_t depressed = xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_DEPRESSED);
    uint32_t latched = xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_LATCHED);
    uint32_t locked = xkb_state_serialize_mods(m_xkbState, XKB_STATE_MODS_LOCKED);
    uint32_t group = xkb_state_serialize_layout(m_xkbState, XKB_STATE_LAYOUT_EFFECTIVE);

    Q_EMIT modifiersChanged(depressed, latched, locked, group);

    if (group != m_currentLayout) {
        m_currentLayout = group;
        Q_EMIT layoutChanged(m_currentLayout);
    }

    return sym;
}

uint32_t VeridianKeyboardState::keysym(uint32_t key) const
{
    return xkb_state_key_get_one_sym(m_xkbState, key + 8);
}

QString VeridianKeyboardState::keyText(uint32_t key) const
{
    char buf[64];
    int len = xkb_state_key_get_utf8(m_xkbState, key + 8, buf, sizeof(buf));
    if (len <= 0)
        return QString();
    return QString::fromUtf8(buf, len);
}

uint32_t VeridianKeyboardState::modifiers() const
{
    uint32_t mods = 0;
    if (xkb_state_mod_index_is_active(m_xkbState, m_modShift, XKB_STATE_MODS_EFFECTIVE))
        mods |= 0x01;
    if (xkb_state_mod_index_is_active(m_xkbState, m_modControl, XKB_STATE_MODS_EFFECTIVE))
        mods |= 0x04;
    if (xkb_state_mod_index_is_active(m_xkbState, m_modAlt, XKB_STATE_MODS_EFFECTIVE))
        mods |= 0x08;
    if (xkb_state_mod_index_is_active(m_xkbState, m_modMeta, XKB_STATE_MODS_EFFECTIVE))
        mods |= 0x40;
    return mods;
}

bool VeridianKeyboardState::isShiftPressed() const
{
    return xkb_state_mod_index_is_active(m_xkbState, m_modShift,
                                          XKB_STATE_MODS_EFFECTIVE);
}

bool VeridianKeyboardState::isControlPressed() const
{
    return xkb_state_mod_index_is_active(m_xkbState, m_modControl,
                                          XKB_STATE_MODS_EFFECTIVE);
}

bool VeridianKeyboardState::isAltPressed() const
{
    return xkb_state_mod_index_is_active(m_xkbState, m_modAlt,
                                          XKB_STATE_MODS_EFFECTIVE);
}

bool VeridianKeyboardState::isMetaPressed() const
{
    return xkb_state_mod_index_is_active(m_xkbState, m_modMeta,
                                          XKB_STATE_MODS_EFFECTIVE);
}

uint32_t VeridianKeyboardState::modifierIndex(const char *name) const
{
    return xkb_keymap_mod_get_index(m_xkbKeymap, name);
}

void VeridianKeyboardState::switchLayout(uint32_t layoutIndex)
{
    /* Cycle to the requested layout group */
    m_currentLayout = layoutIndex;
    /* xkb_state layout switching would be done via xkb_state_update_mask */
}

uint32_t VeridianKeyboardState::currentLayout() const { return m_currentLayout; }

uint32_t VeridianKeyboardState::layoutCount() const
{
    return xkb_keymap_num_layouts(m_xkbKeymap);
}

QString VeridianKeyboardState::layoutName(uint32_t index) const
{
    const char *name = xkb_keymap_layout_get_name(m_xkbKeymap, index);
    return name ? QString::fromUtf8(name) : QString();
}

struct xkb_keymap *VeridianKeyboardState::keymap() const { return m_xkbKeymap; }
struct xkb_state *VeridianKeyboardState::xkbState() const { return m_xkbState; }
int VeridianKeyboardState::keymapFd() const { return m_keymapFd; }
uint32_t VeridianKeyboardState::keymapSize() const { return m_keymapSize; }

/* ========================================================================= */
/* VeridianInputBackend                                                      */
/* ========================================================================= */

/* libinput interface: open device via logind TakeDevice */
static const struct libinput_interface s_libinputInterface = {
    VeridianInputBackend::openRestricted,
    VeridianInputBackend::closeRestricted,
};

VeridianInputBackend::VeridianInputBackend(VeridianSession *session,
                                           QObject *parent)
    : QObject(parent)
    , m_session(session)
    , m_libinput(nullptr)
    , m_keyboardState(nullptr)
    , m_notifier(nullptr)
    , m_seatName(QStringLiteral("seat0"))
{
}

VeridianInputBackend::~VeridianInputBackend()
{
    delete m_notifier;
    delete m_keyboardState;

    if (m_libinput)
        libinput_unref(m_libinput);
}

bool VeridianInputBackend::initialize()
{
    /* Create libinput context with our custom open/close callbacks.
     * The callbacks use VeridianSession::takeDevice() to open /dev/input/
     * nodes via logind, ensuring proper permissions. */
    m_libinput = libinput_path_create_context(&s_libinputInterface, this);
    if (!m_libinput) {
        qWarning("VeridianInputBackend: libinput_path_create_context failed");
        return false;
    }

    /* Enumerate and add input devices.
     * On VeridianOS, /dev/input/ is populated by the kernel's input
     * subsystem for PS/2 keyboard, VirtIO input, and USB HID devices. */
    QDir inputDir(QStringLiteral("/dev/input"));
    QStringList filters;
    filters << QStringLiteral("event*");
    QStringList entries = inputDir.entryList(filters, QDir::System, QDir::Name);

    for (const QString &entry : entries) {
        QString path = QStringLiteral("/dev/input/") + entry;
        struct libinput_device *dev = libinput_path_add_device(m_libinput,
                                                                path.toUtf8().constData());
        if (dev) {
            VeridianInputDevice info = createDeviceInfo(dev);
            applyDeviceConfig(info);
            m_devices.insert(info.sysPath, info);

            qDebug("VeridianInputBackend: added device %s (%s) [kb=%d ptr=%d touch=%d]",
                   qPrintable(info.name), qPrintable(info.sysPath),
                   info.hasKeyboard, info.hasPointer, info.hasTouch);
        }
    }

    /* Initialize keyboard state (xkbcommon) */
    m_keyboardState = new VeridianKeyboardState(this);
    if (!m_keyboardState->initialize()) {
        qWarning("VeridianInputBackend: keyboard state init failed (non-fatal)");
    }

    /* Set up event monitoring on libinput fd */
    int fd = libinput_get_fd(m_libinput);
    m_notifier = new QSocketNotifier(fd, QSocketNotifier::Read, this);
    connect(m_notifier, &QSocketNotifier::activated,
            this, &VeridianInputBackend::onLibinputEvent);

    qDebug("VeridianInputBackend: initialized with %d device(s)", m_devices.size());
    return true;
}

void VeridianInputBackend::suspend()
{
    libinput_suspend(m_libinput);
    if (m_notifier)
        m_notifier->setEnabled(false);
}

void VeridianInputBackend::resume()
{
    libinput_resume(m_libinput);
    if (m_notifier)
        m_notifier->setEnabled(true);
}

int VeridianInputBackend::openRestricted(const char *path, int flags, void *userData)
{
    auto *self = static_cast<VeridianInputBackend *>(userData);

    /* Try logind TakeDevice first */
    if (self->m_session) {
        int fd = self->m_session->takeDevice(QString::fromUtf8(path));
        if (fd >= 0)
            return fd;
    }

    /* Fallback: direct open */
    int fd = ::open(path, flags);
    if (fd < 0) {
        qWarning("VeridianInputBackend: open(%s) failed: %s", path, strerror(errno));
        return -errno;
    }

    return fd;
}

void VeridianInputBackend::closeRestricted(int fd, void *userData)
{
    auto *self = static_cast<VeridianInputBackend *>(userData);

    if (self->m_session) {
        self->m_session->releaseDevice(fd);
    } else {
        ::close(fd);
    }
}

VeridianInputDevice VeridianInputBackend::createDeviceInfo(struct libinput_device *dev)
{
    VeridianInputDevice info;
    info.device = dev;
    info.name = QString::fromUtf8(libinput_device_get_name(dev));
    info.sysPath = QString::fromUtf8(
        libinput_device_get_sysname(dev));
    info.vendorId = libinput_device_get_id_vendor(dev);
    info.productId = libinput_device_get_id_product(dev);

    info.hasKeyboard = libinput_device_has_capability(dev, LIBINPUT_CAP_KEYBOARD);
    info.hasPointer = libinput_device_has_capability(dev, LIBINPUT_CAP_POINTER);
    info.hasTouch = libinput_device_has_capability(dev, LIBINPUT_CAP_TOUCH);
    info.hasTablet = libinput_device_has_capability(dev, LIBINPUT_CAP_TABLET_TOOL);
    info.hasTabletPad = libinput_device_has_capability(dev, LIBINPUT_CAP_TABLET_PAD);
    info.hasGesture = libinput_device_has_capability(dev, LIBINPUT_CAP_GESTURE);
    info.hasSwitch = libinput_device_has_capability(dev, LIBINPUT_CAP_SWITCH);

    info.naturalScroll = false;
    info.leftHanded = false;
    info.tapToClick = false;
    info.tapAndDrag = false;
    info.middleEmulation = false;
    info.pointerAcceleration = 0.0;
    info.accelProfile = LIBINPUT_CONFIG_ACCEL_PROFILE_ADAPTIVE;
    info.scrollMethod = LIBINPUT_CONFIG_SCROLL_NO_SCROLL;
    info.clickMethod = LIBINPUT_CONFIG_CLICK_METHOD_NONE;
    info.enabled = true;

    return info;
}

void VeridianInputBackend::applyDeviceConfig(VeridianInputDevice &info)
{
    struct libinput_device *dev = info.device;

    /* Apply sensible defaults for touchpad devices */
    if (info.hasPointer) {
        if (libinput_device_config_tap_get_finger_count(dev) > 0) {
            info.tapToClick = true;
            libinput_device_config_tap_set_enabled(dev, LIBINPUT_CONFIG_TAP_ENABLED);
            libinput_device_config_tap_set_drag_enabled(dev,
                LIBINPUT_CONFIG_DRAG_ENABLED);
            info.tapAndDrag = true;
        }

        if (libinput_device_config_scroll_has_natural_scroll(dev)) {
            /* Default: natural scroll off for mice, on for touchpads */
            bool isTouchpad = (libinput_device_config_tap_get_finger_count(dev) > 0);
            info.naturalScroll = isTouchpad;
            libinput_device_config_scroll_set_natural_scroll_enabled(dev,
                info.naturalScroll);
        }

        libinput_device_config_accel_set_speed(dev, info.pointerAcceleration);
    }

    /* Apply middle-click emulation for trackpoints */
    if (libinput_device_config_middle_emulation_is_available(dev)) {
        libinput_device_config_middle_emulation_set_enabled(dev,
            LIBINPUT_CONFIG_MIDDLE_EMULATION_ENABLED);
        info.middleEmulation = true;
    }
}

void VeridianInputBackend::onLibinputEvent()
{
    processEvents();
}

void VeridianInputBackend::processEvents()
{
    if (libinput_dispatch(m_libinput) != 0) {
        qWarning("VeridianInputBackend: libinput_dispatch failed");
        return;
    }

    struct libinput_event *event;
    while ((event = libinput_get_event(m_libinput)) != nullptr) {
        enum libinput_event_type type = libinput_event_get_type(event);

        switch (type) {
        /* ----- Keyboard ----- */
        case LIBINPUT_EVENT_KEYBOARD_KEY:
            handleKeyboardEvent(event);
            break;

        /* ----- Pointer ----- */
        case LIBINPUT_EVENT_POINTER_MOTION:
            handlePointerMotionEvent(event);
            break;
        case LIBINPUT_EVENT_POINTER_MOTION_ABSOLUTE:
            handlePointerMotionAbsoluteEvent(event);
            break;
        case LIBINPUT_EVENT_POINTER_BUTTON:
            handlePointerButtonEvent(event);
            break;
        case LIBINPUT_EVENT_POINTER_AXIS:
            handlePointerAxisEvent(event);
            break;

        /* ----- Touch ----- */
        case LIBINPUT_EVENT_TOUCH_DOWN:
            handleTouchDownEvent(event);
            break;
        case LIBINPUT_EVENT_TOUCH_UP:
            handleTouchUpEvent(event);
            break;
        case LIBINPUT_EVENT_TOUCH_MOTION:
            handleTouchMotionEvent(event);
            break;
        case LIBINPUT_EVENT_TOUCH_FRAME:
            handleTouchFrameEvent(event);
            break;
        case LIBINPUT_EVENT_TOUCH_CANCEL:
            handleTouchCancelEvent(event);
            break;

        /* ----- Device management ----- */
        case LIBINPUT_EVENT_DEVICE_ADDED:
            handleDeviceAddedEvent(event);
            break;
        case LIBINPUT_EVENT_DEVICE_REMOVED:
            handleDeviceRemovedEvent(event);
            break;

        default:
            /* Ignore unhandled event types (gestures, tablet, switch) */
            break;
        }

        libinput_event_destroy(event);
    }
}

/* ========================================================================= */
/* Keyboard event handling                                                   */
/* ========================================================================= */

void VeridianInputBackend::handleKeyboardEvent(struct libinput_event *event)
{
    struct libinput_event_keyboard *kbEvent =
        libinput_event_get_keyboard_event(event);

    uint32_t key = libinput_event_keyboard_get_key(kbEvent);
    uint32_t time = libinput_event_keyboard_get_time(kbEvent);
    bool pressed = (libinput_event_keyboard_get_key_state(kbEvent) ==
                    LIBINPUT_KEY_STATE_PRESSED);

    /* Update xkb state and get keysym */
    uint32_t sym = 0;
    if (m_keyboardState) {
        sym = m_keyboardState->processKey(key, pressed);
    }

    if (pressed) {
        Q_EMIT keyPressed(key, sym, time);
    } else {
        Q_EMIT keyReleased(key, sym, time);
    }
}

/* ========================================================================= */
/* Pointer event handling                                                    */
/* ========================================================================= */

void VeridianInputBackend::handlePointerMotionEvent(struct libinput_event *event)
{
    struct libinput_event_pointer *ptrEvent =
        libinput_event_get_pointer_event(event);

    double dx = libinput_event_pointer_get_dx(ptrEvent);
    double dy = libinput_event_pointer_get_dy(ptrEvent);
    double dxUnaccel = libinput_event_pointer_get_dx_unaccelerated(ptrEvent);
    double dyUnaccel = libinput_event_pointer_get_dy_unaccelerated(ptrEvent);
    uint32_t time = libinput_event_pointer_get_time(ptrEvent);

    Q_EMIT pointerMotion(QPointF(dx, dy), QPointF(dxUnaccel, dyUnaccel), time);
}

void VeridianInputBackend::handlePointerMotionAbsoluteEvent(
    struct libinput_event *event)
{
    struct libinput_event_pointer *ptrEvent =
        libinput_event_get_pointer_event(event);

    /* Absolute coordinates are normalized [0.0, 1.0].
     * KWin will multiply by output size. */
    double x = libinput_event_pointer_get_absolute_x_transformed(ptrEvent, 1);
    double y = libinput_event_pointer_get_absolute_y_transformed(ptrEvent, 1);
    uint32_t time = libinput_event_pointer_get_time(ptrEvent);

    Q_EMIT pointerMotionAbsolute(QPointF(x, y), time);
}

void VeridianInputBackend::handlePointerButtonEvent(struct libinput_event *event)
{
    struct libinput_event_pointer *ptrEvent =
        libinput_event_get_pointer_event(event);

    uint32_t button = libinput_event_pointer_get_button(ptrEvent);
    uint32_t time = libinput_event_pointer_get_time(ptrEvent);
    bool pressed = (libinput_event_pointer_get_button_state(ptrEvent) ==
                    LIBINPUT_BUTTON_STATE_PRESSED);

    if (pressed) {
        Q_EMIT pointerButtonPressed(button, time);
    } else {
        Q_EMIT pointerButtonReleased(button, time);
    }
}

void VeridianInputBackend::handlePointerAxisEvent(struct libinput_event *event)
{
    struct libinput_event_pointer *ptrEvent =
        libinput_event_get_pointer_event(event);
    uint32_t time = libinput_event_pointer_get_time(ptrEvent);

    if (libinput_event_pointer_has_axis(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL)) {
        double value = libinput_event_pointer_get_axis_value(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL);
        int32_t discrete = libinput_event_pointer_get_axis_value_discrete(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_VERTICAL);
        Q_EMIT pointerAxisVertical(value, discrete, time);
    }

    if (libinput_event_pointer_has_axis(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL)) {
        double value = libinput_event_pointer_get_axis_value(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL);
        int32_t discrete = libinput_event_pointer_get_axis_value_discrete(ptrEvent,
            LIBINPUT_POINTER_AXIS_SCROLL_HORIZONTAL);
        Q_EMIT pointerAxisHorizontal(value, discrete, time);
    }
}

/* ========================================================================= */
/* Touch event handling                                                      */
/* ========================================================================= */

void VeridianInputBackend::handleTouchDownEvent(struct libinput_event *event)
{
    struct libinput_event_touch *touchEvent =
        libinput_event_get_touch_event(event);

    int32_t id = libinput_event_touch_get_slot(touchEvent);
    double x = libinput_event_touch_get_x_transformed(touchEvent, 1);
    double y = libinput_event_touch_get_y_transformed(touchEvent, 1);
    uint32_t time = libinput_event_touch_get_time(touchEvent);

    Q_EMIT touchDown(id, QPointF(x, y), time);
}

void VeridianInputBackend::handleTouchUpEvent(struct libinput_event *event)
{
    struct libinput_event_touch *touchEvent =
        libinput_event_get_touch_event(event);

    int32_t id = libinput_event_touch_get_slot(touchEvent);
    uint32_t time = libinput_event_touch_get_time(touchEvent);

    Q_EMIT touchUp(id, time);
}

void VeridianInputBackend::handleTouchMotionEvent(struct libinput_event *event)
{
    struct libinput_event_touch *touchEvent =
        libinput_event_get_touch_event(event);

    int32_t id = libinput_event_touch_get_slot(touchEvent);
    double x = libinput_event_touch_get_x_transformed(touchEvent, 1);
    double y = libinput_event_touch_get_y_transformed(touchEvent, 1);
    uint32_t time = libinput_event_touch_get_time(touchEvent);

    Q_EMIT touchMotion(id, QPointF(x, y), time);
}

void VeridianInputBackend::handleTouchFrameEvent(struct libinput_event *event)
{
    Q_UNUSED(event);
    Q_EMIT touchFrame();
}

void VeridianInputBackend::handleTouchCancelEvent(struct libinput_event *event)
{
    Q_UNUSED(event);
    Q_EMIT touchCancel();
}

/* ========================================================================= */
/* Device hot-plug                                                           */
/* ========================================================================= */

void VeridianInputBackend::handleDeviceAddedEvent(struct libinput_event *event)
{
    struct libinput_device *dev = libinput_event_get_device(event);
    VeridianInputDevice info = createDeviceInfo(dev);
    applyDeviceConfig(info);
    m_devices.insert(info.sysPath, info);

    qDebug("VeridianInputBackend: device added: %s", qPrintable(info.name));
    Q_EMIT deviceAdded(info);
}

void VeridianInputBackend::handleDeviceRemovedEvent(struct libinput_event *event)
{
    struct libinput_device *dev = libinput_event_get_device(event);
    QString sysPath = QString::fromUtf8(libinput_device_get_sysname(dev));

    if (m_devices.contains(sysPath)) {
        qDebug("VeridianInputBackend: device removed: %s",
               qPrintable(m_devices[sysPath].name));
        m_devices.remove(sysPath);
        Q_EMIT deviceRemoved(sysPath);
    }
}

/* ========================================================================= */
/* Device queries and configuration                                          */
/* ========================================================================= */

QVector<VeridianInputDevice> VeridianInputBackend::devices() const
{
    QVector<VeridianInputDevice> result;
    for (auto it = m_devices.begin(); it != m_devices.end(); ++it)
        result.append(it.value());
    return result;
}

VeridianInputDevice *VeridianInputBackend::deviceByPath(const QString &sysPath)
{
    auto it = m_devices.find(sysPath);
    return (it != m_devices.end()) ? &it.value() : nullptr;
}

void VeridianInputBackend::setPointerAcceleration(const QString &sysPath,
                                                   double accel)
{
    VeridianInputDevice *dev = deviceByPath(sysPath);
    if (dev && dev->hasPointer) {
        libinput_device_config_accel_set_speed(dev->device, accel);
        dev->pointerAcceleration = accel;
    }
}

void VeridianInputBackend::setNaturalScroll(const QString &sysPath, bool enabled)
{
    VeridianInputDevice *dev = deviceByPath(sysPath);
    if (dev && dev->hasPointer) {
        libinput_device_config_scroll_set_natural_scroll_enabled(dev->device, enabled);
        dev->naturalScroll = enabled;
    }
}

void VeridianInputBackend::setTapToClick(const QString &sysPath, bool enabled)
{
    VeridianInputDevice *dev = deviceByPath(sysPath);
    if (dev && dev->hasPointer) {
        libinput_device_config_tap_set_enabled(dev->device,
            enabled ? LIBINPUT_CONFIG_TAP_ENABLED : LIBINPUT_CONFIG_TAP_DISABLED);
        dev->tapToClick = enabled;
    }
}

void VeridianInputBackend::setLeftHanded(const QString &sysPath, bool enabled)
{
    VeridianInputDevice *dev = deviceByPath(sysPath);
    if (dev) {
        libinput_device_config_left_handed_set(dev->device, enabled);
        dev->leftHanded = enabled;
    }
}

VeridianKeyboardState *VeridianInputBackend::keyboardState() const
{
    return m_keyboardState;
}

void VeridianInputBackend::setKeyboardLayout(const QString &layout,
                                              const QString &variant)
{
    if (m_keyboardState) {
        m_keyboardState->initialize(layout, variant);
    }
}

QString VeridianInputBackend::seatName() const { return m_seatName; }

} /* namespace KWin */
