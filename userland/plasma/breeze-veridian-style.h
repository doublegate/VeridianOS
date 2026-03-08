/*
 * VeridianOS -- breeze-veridian-style.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Breeze Qt widget style backend for VeridianOS.  Implements the full
 * Breeze visual design for Qt widgets including buttons, scrollbars,
 * frames, tabs, sliders, progress bars, and other controls.
 *
 * Responsibilities:
 *   - Widget painting primitives (drawPrimitive, drawControl, drawComplexControl)
 *   - Color role mappings from Breeze color scheme
 *   - Hover/focus/press state animations
 *   - High-DPI scaling support (device pixel ratio aware)
 *   - Sub-control positioning (scrollbar arrows, slider groove/handle)
 *   - Size hints and metrics for all widget types
 *
 * This style plugin is loaded by Qt when the user selects "breeze"
 * as the widget style via kdeglobals or System Settings.
 */

#ifndef BREEZE_VERIDIAN_STYLE_H
#define BREEZE_VERIDIAN_STYLE_H

#include <QObject>
#include <QProxyStyle>
#include <QPainter>
#include <QStyleOption>
#include <QColor>
#include <QHash>
#include <QTimer>
#include <QPointer>
#include <QPropertyAnimation>
#include <QParallelAnimationGroup>

/* Forward declarations */
class QWidget;
class QAbstractScrollArea;
class QToolBar;

namespace Breeze {

/* ========================================================================= */
/* BreezeMetrics -- standard dimensions for Breeze widgets                   */
/* ========================================================================= */

namespace Metrics {
    /* Frame */
    static constexpr int FrameRadius        = 3;
    static constexpr int FrameWidth         = 1;
    static constexpr int FrameOutlineWidth  = 1;

    /* Layout */
    static constexpr int LayoutTopMargin    = 10;
    static constexpr int LayoutBottomMargin = 10;
    static constexpr int LayoutLeftMargin   = 10;
    static constexpr int LayoutRightMargin  = 10;
    static constexpr int LayoutSpacing      = 6;

    /* Push button */
    static constexpr int ButtonMinWidth     = 80;
    static constexpr int ButtonMinHeight    = 30;
    static constexpr int ButtonPadding      = 8;
    static constexpr int ButtonRadius       = 3;
    static constexpr int ButtonFocusWidth   = 2;

    /* Tool button */
    static constexpr int ToolButtonMinWidth = 22;
    static constexpr int ToolButtonPadding  = 4;

    /* Check box / Radio button */
    static constexpr int CheckBoxSize       = 20;
    static constexpr int CheckMarkSize      = 10;
    static constexpr int RadioMarkSize      = 8;
    static constexpr int CheckBoxSpacing    = 6;

    /* Scrollbar */
    static constexpr int ScrollBarWidth     = 14;
    static constexpr int ScrollBarMinSlider = 20;
    static constexpr int ScrollBarRadius    = 7;

    /* Slider */
    static constexpr int SliderGrooveHeight = 4;
    static constexpr int SliderHandleSize   = 20;
    static constexpr int SliderTickSize     = 6;
    static constexpr int SliderTickSpacing  = 2;

    /* Progress bar */
    static constexpr int ProgressBarHeight  = 6;
    static constexpr int ProgressBarRadius  = 3;

    /* Tab bar */
    static constexpr int TabBarHeight       = 30;
    static constexpr int TabBarPadding      = 8;
    static constexpr int TabBarOverlap      = 1;
    static constexpr int TabBarRadius       = 3;

    /* Menu */
    static constexpr int MenuItemHeight     = 30;
    static constexpr int MenuItemPadding    = 8;
    static constexpr int MenuIconSize       = 16;
    static constexpr int MenuSeparatorHeight = 1;
    static constexpr int MenuSeparatorMargin = 4;

    /* Combo box */
    static constexpr int ComboBoxMinHeight  = 30;
    static constexpr int ComboBoxArrowSize  = 12;
    static constexpr int ComboBoxPadding    = 6;

    /* Spin box */
    static constexpr int SpinBoxMinHeight   = 30;
    static constexpr int SpinBoxArrowSize   = 10;

    /* Group box */
    static constexpr int GroupBoxTitleMargin = 4;

    /* Header */
    static constexpr int HeaderMargin       = 4;
    static constexpr int HeaderArrowSize    = 10;

    /* Tooltip */
    static constexpr int TooltipPadding     = 6;
    static constexpr int TooltipRadius      = 3;

    /* Animation */
    static constexpr int AnimationDuration  = 150;  /* ms */
    static constexpr int FocusDuration      = 250;  /* ms */
    static constexpr int HoverDuration      = 150;  /* ms */

    /* Shadow */
    static constexpr int ShadowSize         = 8;
    static constexpr int ShadowOffset       = 2;
}

/* ========================================================================= */
/* BreezeAnimation -- hover/focus/press transition animation                 */
/* ========================================================================= */

/**
 * Manages smooth opacity transitions for widget hover, focus, and press
 * states.  Each tracked widget gets an animation entry that interpolates
 * between 0.0 (inactive) and 1.0 (active) over the configured duration.
 */
class BreezeAnimationData : public QObject
{
    Q_OBJECT
    Q_PROPERTY(qreal opacity READ opacity WRITE setOpacity)

public:
    explicit BreezeAnimationData(QObject *parent = nullptr);
    ~BreezeAnimationData() override;

    qreal opacity() const { return m_opacity; }
    void setOpacity(qreal value);

    void startAnimation(qreal target, int duration);
    void stopAnimation();
    bool isAnimating() const;

Q_SIGNALS:
    void opacityChanged();

private:
    qreal m_opacity;
    QPropertyAnimation *m_animation;
};

/* ========================================================================= */
/* BreezeAnimationEngine -- tracks animations for multiple widgets           */
/* ========================================================================= */

class BreezeAnimationEngine : public QObject
{
    Q_OBJECT

public:
    explicit BreezeAnimationEngine(QObject *parent = nullptr);
    ~BreezeAnimationEngine() override;

    void registerWidget(QWidget *widget);
    void unregisterWidget(QWidget *widget);

    BreezeAnimationData *animationData(const QWidget *widget) const;
    qreal hoverOpacity(const QWidget *widget) const;
    qreal focusOpacity(const QWidget *widget) const;

    void updateHover(QWidget *widget, bool hovered);
    void updateFocus(QWidget *widget, bool focused);

private:
    struct WidgetAnimation {
        BreezeAnimationData *hover;
        BreezeAnimationData *focus;
    };

    QHash<const QWidget *, WidgetAnimation> m_animations;
};

/* ========================================================================= */
/* BreezeStyle -- the main Qt style implementation                           */
/* ========================================================================= */

/**
 * Breeze widget style for VeridianOS.
 *
 * Inherits QProxyStyle to provide Breeze-themed rendering of all standard
 * Qt widgets.  Uses the Fusion style as base for any unimplemented controls.
 *
 * High-DPI support: All pixel dimensions are multiplied by the device pixel
 * ratio at paint time.  The Metrics constants are logical pixels.
 */
class BreezeStyle : public QProxyStyle
{
    Q_OBJECT

public:
    explicit BreezeStyle();
    ~BreezeStyle() override;

    /* ----- Primitive elements ----- */
    void drawPrimitive(PrimitiveElement element,
                       const QStyleOption *option,
                       QPainter *painter,
                       const QWidget *widget) const override;

    /* ----- Control elements ----- */
    void drawControl(ControlElement element,
                     const QStyleOption *option,
                     QPainter *painter,
                     const QWidget *widget) const override;

    /* ----- Complex controls ----- */
    void drawComplexControl(ComplexControl control,
                            const QStyleOptionComplex *option,
                            QPainter *painter,
                            const QWidget *widget) const override;

    /* ----- Sub-control rectangles ----- */
    QRect subControlRect(ComplexControl control,
                         const QStyleOptionComplex *option,
                         SubControl subControl,
                         const QWidget *widget) const override;

    /* ----- Size hints ----- */
    QSize sizeFromContents(ContentsType type,
                           const QStyleOption *option,
                           const QSize &size,
                           const QWidget *widget) const override;

    /* ----- Pixel metrics ----- */
    int pixelMetric(PixelMetric metric,
                    const QStyleOption *option,
                    const QWidget *widget) const override;

    /* ----- Style hints ----- */
    int styleHint(StyleHint hint,
                  const QStyleOption *option,
                  const QWidget *widget,
                  QStyleHintReturn *returnData) const override;

    /* ----- Sub-element rectangles ----- */
    QRect subElementRect(SubElement element,
                         const QStyleOption *option,
                         const QWidget *widget) const override;

    /* ----- Icon ----- */
    QIcon standardIcon(StandardPixmap standardIcon,
                       const QStyleOption *option,
                       const QWidget *widget) const override;

    /* ----- Animation integration ----- */
    void polish(QWidget *widget) override;
    void unpolish(QWidget *widget) override;
    void polish(QApplication *app) override;
    void polish(QPalette &palette) override;

    /* ----- Event filter for animations ----- */
    bool eventFilter(QObject *obj, QEvent *event) override;

private:
    /* ----- Drawing helpers ----- */
    void drawRoundedRect(QPainter *painter, const QRectF &rect,
                         qreal radius, const QColor &fill,
                         const QColor &border = QColor()) const;

    void drawFocusRing(QPainter *painter, const QRectF &rect,
                       qreal radius, const QColor &color,
                       qreal opacity = 1.0) const;

    void drawArrow(QPainter *painter, const QRectF &rect,
                   Qt::ArrowType arrow, const QColor &color) const;

    void drawCheckMark(QPainter *painter, const QRectF &rect,
                       const QColor &color) const;

    /* ----- Breeze primitive painters ----- */
    void drawPanelButton(QPainter *painter, const QStyleOption *option,
                         const QWidget *widget) const;
    void drawPanelFrame(QPainter *painter, const QStyleOption *option,
                        const QWidget *widget) const;
    void drawIndicatorCheckBox(QPainter *painter, const QStyleOption *option,
                               const QWidget *widget) const;
    void drawIndicatorRadio(QPainter *painter, const QStyleOption *option,
                            const QWidget *widget) const;
    void drawScrollBar(QPainter *painter, const QStyleOptionComplex *option,
                       const QWidget *widget) const;
    void drawSlider(QPainter *painter, const QStyleOptionComplex *option,
                    const QWidget *widget) const;
    void drawProgressBar(QPainter *painter, const QStyleOption *option,
                         const QWidget *widget) const;
    void drawTabBarTab(QPainter *painter, const QStyleOption *option,
                       const QWidget *widget) const;
    void drawMenuItem(QPainter *painter, const QStyleOption *option,
                      const QWidget *widget) const;
    void drawComboBox(QPainter *painter, const QStyleOptionComplex *option,
                      const QWidget *widget) const;
    void drawSpinBox(QPainter *painter, const QStyleOptionComplex *option,
                     const QWidget *widget) const;
    void drawToolButton(QPainter *painter, const QStyleOptionComplex *option,
                        const QWidget *widget) const;
    void drawGroupBox(QPainter *painter, const QStyleOptionComplex *option,
                      const QWidget *widget) const;
    void drawHeaderSection(QPainter *painter, const QStyleOption *option,
                           const QWidget *widget) const;

    /* ----- Color helpers ----- */
    QColor buttonColor(const QStyleOption *option,
                       const QWidget *widget) const;
    QColor hoverColor(const QStyleOption *option) const;
    QColor focusColor(const QStyleOption *option) const;
    QColor separatorColor(const QStyleOption *option) const;
    QColor shadowColor(const QStyleOption *option) const;

    /* ----- Scaling helper ----- */
    qreal dpiScale(const QWidget *widget) const;

    /* ----- Animation engine ----- */
    BreezeAnimationEngine *m_animationEngine;
};

/* ========================================================================= */
/* Plugin factory                                                            */
/* ========================================================================= */

class BreezeStylePlugin : public QStylePlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QStyleFactoryInterface_iid
                      FILE "breeze.json")

public:
    QStyle *create(const QString &key) override;
    QStringList keys() const;
};

} /* namespace Breeze */

#endif /* BREEZE_VERIDIAN_STYLE_H */
