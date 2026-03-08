/*
 * VeridianOS -- breeze-veridian-decoration.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KWin window decoration plugin implementing the Breeze design language.
 * Provides title bar rendering, window buttons (close, maximize, minimize,
 * on-all-desktops), and border sizing for KWin's KDecoration2 framework.
 *
 * Responsibilities:
 *   - Title bar height, font, and color configuration
 *   - Window button rendering (close=red, maximize=green, minimize=yellow,
 *     on-all-desktops=blue, keep-above, keep-below, shade)
 *   - Button hover/press animations with smooth opacity transitions
 *   - Active vs. inactive window differentiation (dimmed title bar)
 *   - Configurable border sizing (none, tiny, normal, large, huge)
 *   - Window shadow rendering (radial gradient)
 *   - High-DPI scaling support
 *
 * This plugin compiles against the KDecoration2 framework from KDE
 * Frameworks 6 and is loaded by KWin at startup.
 */

#ifndef BREEZE_VERIDIAN_DECORATION_H
#define BREEZE_VERIDIAN_DECORATION_H

#include <QObject>
#include <QPainter>
#include <QColor>
#include <QFont>
#include <QRectF>
#include <QPointF>
#include <QSizeF>
#include <QTimer>
#include <QPropertyAnimation>
#include <QVariantAnimation>
#include <QSharedPointer>

/* KDecoration2 API headers (from KDE Frameworks 6 build tree) */
#include <KDecoration2/Decoration>
#include <KDecoration2/DecorationButton>
#include <KDecoration2/DecorationButtonGroup>
#include <KDecoration2/DecorationSettings>
#include <KDecoration2/DecoratedClient>

namespace Breeze {

/* ========================================================================= */
/* Decoration metrics                                                        */
/* ========================================================================= */

namespace DecorationMetrics {
    /* Title bar */
    static constexpr int TitleBarHeight         = 30;
    static constexpr int TitleBarTopMargin      = 4;
    static constexpr int TitleBarSideMargin     = 8;
    static constexpr int TitleBarFontSize       = 10;

    /* Buttons */
    static constexpr int ButtonSize             = 18;
    static constexpr int ButtonSpacing          = 4;
    static constexpr int ButtonMargin           = 6;
    static constexpr int ButtonIconSize         = 10;
    static constexpr int ButtonHoverRadius      = 9;

    /* Borders */
    static constexpr int BorderWidthNone        = 0;
    static constexpr int BorderWidthTiny        = 1;
    static constexpr int BorderWidthNormal      = 4;
    static constexpr int BorderWidthLarge       = 8;
    static constexpr int BorderWidthHuge        = 12;

    /* Corners */
    static constexpr int CornerRadius           = 4;
    static constexpr int CornerRadiusMaximized  = 0;

    /* Shadow */
    static constexpr int ShadowSize             = 32;
    static constexpr int ShadowOffset           = 4;
    static constexpr qreal ShadowStrength       = 0.4;

    /* Animation */
    static constexpr int HoverDuration          = 150;  /* ms */
    static constexpr int PressDuration          = 100;  /* ms */
    static constexpr int ActiveChangeDuration   = 250;  /* ms */
}

/* ========================================================================= */
/* Border size enum                                                          */
/* ========================================================================= */

enum class BorderSize {
    None,
    Tiny,
    Normal,
    Large,
    Huge
};

/* ========================================================================= */
/* BreezeButton -- individual window decoration button                       */
/* ========================================================================= */

/**
 * Represents a single window decoration button (close, maximize, etc.).
 *
 * Each button has:
 *   - A base color determined by its type (close=red, etc.)
 *   - Hover/press opacity animations
 *   - An icon drawn as a simple geometric shape (X, square, line, etc.)
 */
class BreezeButton : public KDecoration2::DecorationButton
{
    Q_OBJECT
    Q_PROPERTY(qreal hoverOpacity READ hoverOpacity WRITE setHoverOpacity)
    Q_PROPERTY(qreal pressOpacity READ pressOpacity WRITE setPressOpacity)

public:
    explicit BreezeButton(KDecoration2::DecorationButtonType type,
                          KDecoration2::Decoration *decoration,
                          QObject *parent = nullptr);
    ~BreezeButton() override;

    /* ----- Painting ----- */
    void paint(QPainter *painter, const QRect &repaintRegion) override;

    /* ----- Animation properties ----- */
    qreal hoverOpacity() const { return m_hoverOpacity; }
    void setHoverOpacity(qreal value);

    qreal pressOpacity() const { return m_pressOpacity; }
    void setPressOpacity(qreal value);

    /* ----- Configuration ----- */
    QColor baseColor() const;
    QColor iconColor() const;

    /* ----- Factory ----- */
    static QPointer<BreezeButton> create(
        KDecoration2::DecorationButtonType type,
        KDecoration2::Decoration *decoration,
        QObject *parent);

private Q_SLOTS:
    void onHoverEntered();
    void onHoverLeft();
    void onPressed();
    void onReleased();

private:
    void drawIcon(QPainter *painter, const QRectF &rect) const;
    void drawCloseIcon(QPainter *painter, const QRectF &rect) const;
    void drawMaximizeIcon(QPainter *painter, const QRectF &rect) const;
    void drawMinimizeIcon(QPainter *painter, const QRectF &rect) const;
    void drawOnAllDesktopsIcon(QPainter *painter, const QRectF &rect) const;
    void drawKeepAboveIcon(QPainter *painter, const QRectF &rect) const;
    void drawKeepBelowIcon(QPainter *painter, const QRectF &rect) const;
    void drawShadeIcon(QPainter *painter, const QRectF &rect) const;
    void drawContextHelpIcon(QPainter *painter, const QRectF &rect) const;

    qreal m_hoverOpacity;
    qreal m_pressOpacity;
    QPropertyAnimation *m_hoverAnimation;
    QPropertyAnimation *m_pressAnimation;
};

/* ========================================================================= */
/* BreezeDecoration -- KWin window decoration                                */
/* ========================================================================= */

/**
 * KWin window decoration implementing the Breeze visual design.
 *
 * Paints the title bar, window borders, and shadow for each decorated
 * window.  The decoration geometry adapts to window state (maximized
 * windows have no rounded corners and minimal borders).
 *
 * Color scheme integration: title bar colors are read from the
 * application's QPalette which is configured by VeridianPlatformTheme.
 */
class BreezeDecoration : public KDecoration2::Decoration
{
    Q_OBJECT

public:
    explicit BreezeDecoration(QObject *parent = nullptr,
                              const QVariantList &args = QVariantList());
    ~BreezeDecoration() override;

    /* ----- KDecoration2 interface ----- */
    void init() override;
    void paint(QPainter *painter, const QRect &repaintRegion) override;

    /* ----- Configuration ----- */
    BorderSize borderSize() const { return m_borderSize; }
    void setBorderSize(BorderSize size);
    int borderWidth() const;
    int titleBarHeight() const;
    bool isMaximized() const;
    qreal cornerRadius() const;
    qreal scaleFactor() const;

    /* ----- Title bar ----- */
    QColor titleBarColor() const;
    QColor titleBarTextColor() const;
    QColor borderColor() const;
    QFont titleBarFont() const;

    /* ----- Shadow ----- */
    QSharedPointer<KDecoration2::DecorationShadow> createShadow();

Q_SIGNALS:
    void borderSizeChanged();

private Q_SLOTS:
    void onActiveChanged();
    void onCaptionChanged();
    void onMaximizedChanged();
    void onWidthChanged();
    void onHeightChanged();
    void onShadingChanged();

private:
    /* ----- Painting helpers ----- */
    void paintTitleBar(QPainter *painter);
    void paintBorders(QPainter *painter);
    void paintCaption(QPainter *painter);

    /* ----- Shadow generation ----- */
    QImage generateShadowImage() const;

    /* ----- Button management ----- */
    void createButtons();
    void updateButtonPositions();

    /* ----- Configuration ----- */
    void loadConfiguration();
    void updateLayout();

    /* ----- State ----- */
    BorderSize m_borderSize;
    QFont m_titleFont;
    qreal m_activeOpacity;
    QVariantAnimation *m_activeAnimation;

    /* Button groups */
    KDecoration2::DecorationButtonGroup *m_leftButtons;
    KDecoration2::DecorationButtonGroup *m_rightButtons;
};

/* ========================================================================= */
/* Plugin factory                                                            */
/* ========================================================================= */

/**
 * KDecoration2 plugin that creates BreezeDecoration instances.
 * Registered via KPluginFactory for KWin's decoration loader.
 */
class BreezeDecorationPlugin : public QObject
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.kde.kdecoration2"
                      FILE "breeze-decoration.json")

public:
    explicit BreezeDecorationPlugin(QObject *parent = nullptr);
    ~BreezeDecorationPlugin() override;
};

} /* namespace Breeze */

#endif /* BREEZE_VERIDIAN_DECORATION_H */
