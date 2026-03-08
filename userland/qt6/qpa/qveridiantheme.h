/*
 * VeridianOS -- qveridiantheme.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * QPlatformTheme implementation for VeridianOS.  Provides default fonts,
 * color palette, icon theme name, and standard pixmaps.
 */

#ifndef QVERIDIANTHEME_H
#define QVERIDIANTHEME_H

#include <QtGui/qpa/qplatformtheme.h>

QT_BEGIN_NAMESPACE

class QVeridianTheme : public QPlatformTheme
{
public:
    QVeridianTheme();
    ~QVeridianTheme() override;

    const QFont *font(Font type = SystemFont) const override;
    const QPalette *palette(Palette type = SystemPalette) const override;

    QVariant themeHint(ThemeHint hint) const override;
    QString standardButtonText(int button) const override;
    QIcon fileIcon(const QFileInfo &fileInfo,
                   QPlatformTheme::IconOptions iconOptions = {}) const override;

    QPlatformMenuBar *createPlatformMenuBar() const override;
    QPlatformMenu *createPlatformMenu() const override;
    QPlatformMenuItem *createPlatformMenuItem() const override;

private:
    void initFonts();
    void initPalette();

    QFont    *m_systemFont   = nullptr;
    QFont    *m_fixedFont    = nullptr;
    QPalette *m_palette      = nullptr;
};

QT_END_NAMESPACE

#endif /* QVERIDIANTHEME_H */
