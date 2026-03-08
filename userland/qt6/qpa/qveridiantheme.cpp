/*
 * VeridianOS -- qveridiantheme.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Default theming for VeridianOS.  Uses DejaVu Sans as the system font,
 * Breeze as the icon theme, and provides a neutral system palette that
 * KDE Plasma will override with its own theme settings.
 */

#include "qveridiantheme.h"

#include <QtGui/QFont>
#include <QtGui/QPalette>
#include <QtGui/QIcon>
#include <QtCore/QFileInfo>

QT_BEGIN_NAMESPACE

QVeridianTheme::QVeridianTheme()
{
    initFonts();
    initPalette();
}

QVeridianTheme::~QVeridianTheme()
{
    delete m_systemFont;
    delete m_fixedFont;
    delete m_palette;
}

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

void QVeridianTheme::initFonts()
{
    /* DejaVu Sans 10pt as the default system font.  This matches what
     * the VeridianOS sysroot provides in /usr/share/fonts/. */
    m_systemFont = new QFont(QStringLiteral("DejaVu Sans"), 10);
    m_systemFont->setStyleHint(QFont::SansSerif);

    /* DejaVu Sans Mono 10pt for fixed-width contexts (terminal, code) */
    m_fixedFont = new QFont(QStringLiteral("DejaVu Sans Mono"), 10);
    m_fixedFont->setStyleHint(QFont::Monospace);
}

void QVeridianTheme::initPalette()
{
    /* Neutral light palette.  KDE Plasma overrides this with Breeze
     * colors via KColorScheme, so these are just sane defaults. */
    m_palette = new QPalette();
    m_palette->setColor(QPalette::Window,          QColor(239, 240, 241));
    m_palette->setColor(QPalette::WindowText,      QColor(35,  38,  41));
    m_palette->setColor(QPalette::Base,             QColor(252, 252, 252));
    m_palette->setColor(QPalette::AlternateBase,    QColor(239, 240, 241));
    m_palette->setColor(QPalette::Text,             QColor(35,  38,  41));
    m_palette->setColor(QPalette::Button,           QColor(239, 240, 241));
    m_palette->setColor(QPalette::ButtonText,       QColor(35,  38,  41));
    m_palette->setColor(QPalette::BrightText,       QColor(255, 255, 255));
    m_palette->setColor(QPalette::Highlight,        QColor(61,  174, 233));
    m_palette->setColor(QPalette::HighlightedText,  QColor(255, 255, 255));
    m_palette->setColor(QPalette::Link,             QColor(41,  128, 185));
    m_palette->setColor(QPalette::LinkVisited,      QColor(127, 140, 141));
    m_palette->setColor(QPalette::ToolTipBase,      QColor(247, 247, 247));
    m_palette->setColor(QPalette::ToolTipText,      QColor(35,  38,  41));
}

/* ========================================================================= */
/* QPlatformTheme interface                                                  */
/* ========================================================================= */

const QFont *QVeridianTheme::font(Font type) const
{
    switch (type) {
    case FixedFont:
        return m_fixedFont;
    default:
        return m_systemFont;
    }
}

const QPalette *QVeridianTheme::palette(Palette type) const
{
    Q_UNUSED(type);
    return m_palette;
}

QVariant QVeridianTheme::themeHint(ThemeHint hint) const
{
    switch (hint) {
    case QPlatformTheme::IconThemeSearchPaths:
        return QStringList{
            QStringLiteral("/usr/share/icons"),
            QStringLiteral("/usr/local/share/icons"),
        };
    case QPlatformTheme::IconThemeName:
        return QStringLiteral("breeze");
    case QPlatformTheme::StyleNames:
        return QStringList{QStringLiteral("breeze"), QStringLiteral("fusion")};
    case QPlatformTheme::SystemIconFallbackThemeName:
        return QStringLiteral("hicolor");
    case QPlatformTheme::DialogButtonBoxButtonsHaveIcons:
        return true;
    case QPlatformTheme::UseFullScreenForPopupMenu:
        return false;
    default:
        return QPlatformTheme::themeHint(hint);
    }
}

QString QVeridianTheme::standardButtonText(int button) const
{
    return QPlatformTheme::standardButtonText(button);
}

QIcon QVeridianTheme::fileIcon(const QFileInfo &fileInfo,
                                QPlatformTheme::IconOptions iconOptions) const
{
    Q_UNUSED(fileInfo);
    Q_UNUSED(iconOptions);
    return QIcon();
}

QPlatformMenuBar *QVeridianTheme::createPlatformMenuBar() const
{
    return nullptr;
}

QPlatformMenu *QVeridianTheme::createPlatformMenu() const
{
    return nullptr;
}

QPlatformMenuItem *QVeridianTheme::createPlatformMenuItem() const
{
    return nullptr;
}

QT_END_NAMESPACE
