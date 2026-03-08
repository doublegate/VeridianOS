/*
 * VeridianOS -- breeze-veridian-style.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Breeze Qt widget style implementation for VeridianOS.
 *
 * Implements the complete Breeze visual design for Qt widgets.  All
 * painting uses QPainter with anti-aliased rendering.  Colors are
 * derived from the application's QPalette (set by VeridianPlatformTheme)
 * so light/dark mode switching works automatically.
 *
 * Animation: hover and focus transitions are managed by BreezeAnimationEngine
 * which tracks per-widget opacity values interpolated over 150ms (hover)
 * and 250ms (focus).
 */

#include "breeze-veridian-style.h"

#include <QApplication>
#include <QAbstractItemView>
#include <QAbstractScrollArea>
#include <QCheckBox>
#include <QComboBox>
#include <QDial>
#include <QDialog>
#include <QDockWidget>
#include <QEvent>
#include <QFocusEvent>
#include <QFormLayout>
#include <QGroupBox>
#include <QHeaderView>
#include <QLabel>
#include <QLineEdit>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QMdiSubWindow>
#include <QPainterPath>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollBar>
#include <QSlider>
#include <QSpinBox>
#include <QSplitter>
#include <QStatusBar>
#include <QTabBar>
#include <QTabWidget>
#include <QTextEdit>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QTreeView>
#include <QWidget>
#include <QWindow>

#include <cmath>

namespace Breeze {

/* ========================================================================= */
/* BreezeAnimationData                                                       */
/* ========================================================================= */

BreezeAnimationData::BreezeAnimationData(QObject *parent)
    : QObject(parent)
    , m_opacity(0.0)
    , m_animation(new QPropertyAnimation(this, "opacity", this))
{
    m_animation->setEasingCurve(QEasingCurve::InOutQuad);
}

BreezeAnimationData::~BreezeAnimationData()
{
}

void BreezeAnimationData::setOpacity(qreal value)
{
    if (qFuzzyCompare(m_opacity, value))
        return;
    m_opacity = value;
    Q_EMIT opacityChanged();
}

void BreezeAnimationData::startAnimation(qreal target, int duration)
{
    m_animation->stop();
    m_animation->setDuration(duration);
    m_animation->setStartValue(m_opacity);
    m_animation->setEndValue(target);
    m_animation->start();
}

void BreezeAnimationData::stopAnimation()
{
    m_animation->stop();
}

bool BreezeAnimationData::isAnimating() const
{
    return m_animation->state() == QAbstractAnimation::Running;
}

/* ========================================================================= */
/* BreezeAnimationEngine                                                     */
/* ========================================================================= */

BreezeAnimationEngine::BreezeAnimationEngine(QObject *parent)
    : QObject(parent)
{
}

BreezeAnimationEngine::~BreezeAnimationEngine()
{
}

void BreezeAnimationEngine::registerWidget(QWidget *widget)
{
    if (m_animations.contains(widget))
        return;

    WidgetAnimation wa;
    wa.hover = new BreezeAnimationData(this);
    wa.focus = new BreezeAnimationData(this);

    /* Trigger widget repaint when animation ticks */
    connect(wa.hover, &BreezeAnimationData::opacityChanged,
            widget, QOverload<>::of(&QWidget::update));
    connect(wa.focus, &BreezeAnimationData::opacityChanged,
            widget, QOverload<>::of(&QWidget::update));

    m_animations.insert(widget, wa);
}

void BreezeAnimationEngine::unregisterWidget(QWidget *widget)
{
    auto it = m_animations.find(widget);
    if (it == m_animations.end())
        return;

    delete it->hover;
    delete it->focus;
    m_animations.erase(it);
}

BreezeAnimationData *BreezeAnimationEngine::animationData(
    const QWidget *widget) const
{
    auto it = m_animations.find(widget);
    return (it != m_animations.end()) ? it->hover : nullptr;
}

qreal BreezeAnimationEngine::hoverOpacity(const QWidget *widget) const
{
    auto it = m_animations.constFind(widget);
    return (it != m_animations.constEnd()) ? it->hover->opacity() : 0.0;
}

qreal BreezeAnimationEngine::focusOpacity(const QWidget *widget) const
{
    auto it = m_animations.constFind(widget);
    return (it != m_animations.constEnd()) ? it->focus->opacity() : 0.0;
}

void BreezeAnimationEngine::updateHover(QWidget *widget, bool hovered)
{
    auto it = m_animations.find(widget);
    if (it == m_animations.end())
        return;
    it->hover->startAnimation(hovered ? 1.0 : 0.0, Metrics::HoverDuration);
}

void BreezeAnimationEngine::updateFocus(QWidget *widget, bool focused)
{
    auto it = m_animations.find(widget);
    if (it == m_animations.end())
        return;
    it->focus->startAnimation(focused ? 1.0 : 0.0, Metrics::FocusDuration);
}

/* ========================================================================= */
/* BreezeStyle -- constructor/destructor                                     */
/* ========================================================================= */

BreezeStyle::BreezeStyle()
    : QProxyStyle(QStyleFactory::create(QStringLiteral("Fusion")))
    , m_animationEngine(new BreezeAnimationEngine(this))
{
}

BreezeStyle::~BreezeStyle()
{
}

/* ========================================================================= */
/* Drawing helpers                                                           */
/* ========================================================================= */

void BreezeStyle::drawRoundedRect(QPainter *painter, const QRectF &rect,
                                   qreal radius, const QColor &fill,
                                   const QColor &border) const
{
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    if (fill.isValid()) {
        painter->setPen(Qt::NoPen);
        painter->setBrush(fill);
        painter->drawRoundedRect(rect, radius, radius);
    }

    if (border.isValid()) {
        painter->setPen(QPen(border, 1.0));
        painter->setBrush(Qt::NoBrush);
        painter->drawRoundedRect(rect.adjusted(0.5, 0.5, -0.5, -0.5),
                                  radius, radius);
    }

    painter->restore();
}

void BreezeStyle::drawFocusRing(QPainter *painter, const QRectF &rect,
                                 qreal radius, const QColor &color,
                                 qreal opacity) const
{
    if (opacity <= 0.0)
        return;

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QColor ringColor = color;
    ringColor.setAlphaF(opacity * 0.6);

    QPen pen(ringColor, Metrics::ButtonFocusWidth);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);

    QRectF focusRect = rect.adjusted(-1, -1, 1, 1);
    painter->drawRoundedRect(focusRect, radius + 1, radius + 1);

    painter->restore();
}

void BreezeStyle::drawArrow(QPainter *painter, const QRectF &rect,
                             Qt::ArrowType arrow, const QColor &color) const
{
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QPainterPath path;
    QPointF center = rect.center();
    qreal size = qMin(rect.width(), rect.height()) * 0.3;

    switch (arrow) {
    case Qt::UpArrow:
        path.moveTo(center.x(), center.y() - size);
        path.lineTo(center.x() - size, center.y() + size * 0.5);
        path.lineTo(center.x() + size, center.y() + size * 0.5);
        break;
    case Qt::DownArrow:
        path.moveTo(center.x(), center.y() + size);
        path.lineTo(center.x() - size, center.y() - size * 0.5);
        path.lineTo(center.x() + size, center.y() - size * 0.5);
        break;
    case Qt::LeftArrow:
        path.moveTo(center.x() - size, center.y());
        path.lineTo(center.x() + size * 0.5, center.y() - size);
        path.lineTo(center.x() + size * 0.5, center.y() + size);
        break;
    case Qt::RightArrow:
        path.moveTo(center.x() + size, center.y());
        path.lineTo(center.x() - size * 0.5, center.y() - size);
        path.lineTo(center.x() - size * 0.5, center.y() + size);
        break;
    default:
        break;
    }

    path.closeSubpath();
    painter->setPen(Qt::NoPen);
    painter->setBrush(color);
    painter->drawPath(path);

    painter->restore();
}

void BreezeStyle::drawCheckMark(QPainter *painter, const QRectF &rect,
                                 const QColor &color) const
{
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QPen pen(color, 2.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);

    /* Draw check mark as two line segments */
    qreal x = rect.x();
    qreal y = rect.y();
    qreal w = rect.width();
    qreal h = rect.height();

    QPainterPath path;
    path.moveTo(x + w * 0.2, y + h * 0.5);
    path.lineTo(x + w * 0.4, y + h * 0.75);
    path.lineTo(x + w * 0.8, y + h * 0.25);
    painter->drawPath(path);

    painter->restore();
}

/* ========================================================================= */
/* Color helpers                                                             */
/* ========================================================================= */

QColor BreezeStyle::buttonColor(const QStyleOption *option,
                                 const QWidget *widget) const
{
    Q_UNUSED(widget);
    QColor base = option->palette.color(QPalette::Button);

    if (!(option->state & QStyle::State_Enabled))
        return base;

    if (option->state & QStyle::State_Sunken)
        return option->palette.color(QPalette::Highlight);

    if (option->state & QStyle::State_MouseOver)
        return base.lighter(110);

    return base;
}

QColor BreezeStyle::hoverColor(const QStyleOption *option) const
{
    return option->palette.color(QPalette::Highlight).lighter(140);
}

QColor BreezeStyle::focusColor(const QStyleOption *option) const
{
    return option->palette.color(QPalette::Highlight);
}

QColor BreezeStyle::separatorColor(const QStyleOption *option) const
{
    return option->palette.color(QPalette::Mid);
}

QColor BreezeStyle::shadowColor(const QStyleOption *option) const
{
    return option->palette.color(QPalette::Shadow);
}

qreal BreezeStyle::dpiScale(const QWidget *widget) const
{
    if (widget && widget->window() && widget->window()->windowHandle())
        return widget->window()->windowHandle()->devicePixelRatio();
    return 1.0;
}

/* ========================================================================= */
/* Primitive elements                                                        */
/* ========================================================================= */

void BreezeStyle::drawPrimitive(PrimitiveElement element,
                                 const QStyleOption *option,
                                 QPainter *painter,
                                 const QWidget *widget) const
{
    switch (element) {
    case PE_PanelButtonCommand:
        drawPanelButton(painter, option, widget);
        return;

    case PE_Frame:
    case PE_FrameDefaultButton:
    case PE_FrameFocusRect:
    case PE_FrameGroupBox:
    case PE_FrameLineEdit:
    case PE_FrameMenu:
    case PE_FrameTabWidget:
    case PE_FrameWindow:
        drawPanelFrame(painter, option, widget);
        return;

    case PE_IndicatorCheckBox:
        drawIndicatorCheckBox(painter, option, widget);
        return;

    case PE_IndicatorRadioButton:
        drawIndicatorRadio(painter, option, widget);
        return;

    case PE_IndicatorArrowDown:
        drawArrow(painter, option->rect, Qt::DownArrow,
                  option->palette.color(QPalette::WindowText));
        return;

    case PE_IndicatorArrowUp:
        drawArrow(painter, option->rect, Qt::UpArrow,
                  option->palette.color(QPalette::WindowText));
        return;

    case PE_IndicatorArrowLeft:
        drawArrow(painter, option->rect, Qt::LeftArrow,
                  option->palette.color(QPalette::WindowText));
        return;

    case PE_IndicatorArrowRight:
        drawArrow(painter, option->rect, Qt::RightArrow,
                  option->palette.color(QPalette::WindowText));
        return;

    case PE_PanelTipLabel:
        drawRoundedRect(painter, QRectF(option->rect), Metrics::TooltipRadius,
                        option->palette.color(QPalette::ToolTipBase),
                        option->palette.color(QPalette::Mid));
        return;

    default:
        QProxyStyle::drawPrimitive(element, option, painter, widget);
        return;
    }
}

/* ========================================================================= */
/* Breeze-specific primitive painters                                        */
/* ========================================================================= */

void BreezeStyle::drawPanelButton(QPainter *painter,
                                   const QStyleOption *option,
                                   const QWidget *widget) const
{
    QRectF rect = QRectF(option->rect).adjusted(1, 1, -1, -1);
    QColor bg = buttonColor(option, widget);
    QColor border = separatorColor(option);

    /* Pressed: darken border */
    if (option->state & QStyle::State_Sunken)
        border = border.darker(120);

    drawRoundedRect(painter, rect, Metrics::ButtonRadius, bg, border);

    /* Focus ring */
    if (option->state & QStyle::State_HasFocus) {
        qreal focusOp = widget ? m_animationEngine->focusOpacity(widget) : 1.0;
        if (focusOp <= 0.0)
            focusOp = 1.0;  /* no animation data -> fully visible */
        drawFocusRing(painter, rect, Metrics::ButtonRadius,
                      focusColor(option), focusOp);
    }
}

void BreezeStyle::drawPanelFrame(QPainter *painter,
                                  const QStyleOption *option,
                                  const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect).adjusted(0.5, 0.5, -0.5, -0.5);
    QColor border = separatorColor(option);

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);
    painter->setPen(QPen(border, Metrics::FrameWidth));
    painter->setBrush(Qt::NoBrush);
    painter->drawRoundedRect(rect, Metrics::FrameRadius, Metrics::FrameRadius);
    painter->restore();
}

void BreezeStyle::drawIndicatorCheckBox(QPainter *painter,
                                         const QStyleOption *option,
                                         const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect);
    bool checked = (option->state & QStyle::State_On);
    bool indeterminate = (option->state & QStyle::State_NoChange);
    bool enabled = (option->state & QStyle::State_Enabled);

    QColor bg = checked ? focusColor(option)
                        : option->palette.color(QPalette::Base);
    QColor border = checked ? focusColor(option).darker(120)
                            : separatorColor(option);

    if (!enabled) {
        bg = option->palette.color(QPalette::Window);
        border = separatorColor(option).lighter(120);
    }

    drawRoundedRect(painter, rect, Metrics::FrameRadius, bg, border);

    if (checked) {
        QRectF markRect = rect.adjusted(3, 3, -3, -3);
        drawCheckMark(painter, markRect,
                      option->palette.color(QPalette::HighlightedText));
    } else if (indeterminate) {
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing, true);
        QRectF dashRect(rect.center().x() - rect.width() * 0.25,
                        rect.center().y() - 1,
                        rect.width() * 0.5, 2);
        painter->setPen(Qt::NoPen);
        painter->setBrush(enabled ? focusColor(option)
                                  : separatorColor(option));
        painter->drawRoundedRect(dashRect, 1, 1);
        painter->restore();
    }
}

void BreezeStyle::drawIndicatorRadio(QPainter *painter,
                                      const QStyleOption *option,
                                      const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect);
    bool checked = (option->state & QStyle::State_On);
    bool enabled = (option->state & QStyle::State_Enabled);

    QColor bg = checked ? focusColor(option)
                        : option->palette.color(QPalette::Base);
    QColor border = checked ? focusColor(option).darker(120)
                            : separatorColor(option);

    if (!enabled) {
        bg = option->palette.color(QPalette::Window);
        border = separatorColor(option).lighter(120);
    }

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    /* Outer circle */
    painter->setPen(QPen(border, 1.0));
    painter->setBrush(bg);
    painter->drawEllipse(rect.adjusted(0.5, 0.5, -0.5, -0.5));

    /* Inner dot for checked state */
    if (checked) {
        QColor dot = option->palette.color(QPalette::HighlightedText);
        painter->setPen(Qt::NoPen);
        painter->setBrush(dot);
        qreal inset = rect.width() * 0.3;
        painter->drawEllipse(rect.adjusted(inset, inset, -inset, -inset));
    }

    painter->restore();
}

/* ========================================================================= */
/* Control elements                                                          */
/* ========================================================================= */

void BreezeStyle::drawControl(ControlElement element,
                               const QStyleOption *option,
                               QPainter *painter,
                               const QWidget *widget) const
{
    switch (element) {
    case CE_ProgressBarGroove:
    case CE_ProgressBarContents:
    case CE_ProgressBarLabel:
        drawProgressBar(painter, option, widget);
        return;

    case CE_TabBarTab:
    case CE_TabBarTabShape:
    case CE_TabBarTabLabel:
        drawTabBarTab(painter, option, widget);
        return;

    case CE_MenuItem:
    case CE_MenuBarItem:
        drawMenuItem(painter, option, widget);
        return;

    case CE_HeaderSection:
        drawHeaderSection(painter, option, widget);
        return;

    default:
        QProxyStyle::drawControl(element, option, painter, widget);
        return;
    }
}

void BreezeStyle::drawProgressBar(QPainter *painter,
                                   const QStyleOption *option,
                                   const QWidget *widget) const
{
    Q_UNUSED(widget);
    const QStyleOptionProgressBar *pbOption =
        qstyleoption_cast<const QStyleOptionProgressBar *>(option);
    if (!pbOption)
        return;

    QRectF groove = QRectF(option->rect);
    groove.setHeight(Metrics::ProgressBarHeight);
    groove.moveCenter(QRectF(option->rect).center());

    /* Groove background */
    drawRoundedRect(painter, groove, Metrics::ProgressBarRadius,
                    option->palette.color(QPalette::Window).darker(110));

    /* Fill */
    if (pbOption->maximum > pbOption->minimum) {
        qreal fraction = static_cast<qreal>(pbOption->progress - pbOption->minimum)
                       / static_cast<qreal>(pbOption->maximum - pbOption->minimum);
        QRectF fill = groove;
        fill.setWidth(groove.width() * fraction);
        drawRoundedRect(painter, fill, Metrics::ProgressBarRadius,
                        focusColor(option));
    }
}

void BreezeStyle::drawTabBarTab(QPainter *painter,
                                 const QStyleOption *option,
                                 const QWidget *widget) const
{
    Q_UNUSED(widget);
    bool selected = (option->state & QStyle::State_Selected);
    QRectF rect = QRectF(option->rect);

    QColor bg = selected ? option->palette.color(QPalette::Base)
                         : option->palette.color(QPalette::Window);
    QColor border = separatorColor(option);

    /* Only round the top corners */
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QPainterPath path;
    qreal r = Metrics::TabBarRadius;
    path.moveTo(rect.bottomLeft());
    path.lineTo(rect.x(), rect.y() + r);
    path.quadTo(rect.x(), rect.y(), rect.x() + r, rect.y());
    path.lineTo(rect.right() - r, rect.y());
    path.quadTo(rect.right(), rect.y(), rect.right(), rect.y() + r);
    path.lineTo(rect.bottomRight());
    path.closeSubpath();

    painter->setPen(QPen(border, 1.0));
    painter->setBrush(bg);
    painter->drawPath(path);

    /* Active tab indicator line */
    if (selected) {
        painter->setPen(Qt::NoPen);
        painter->setBrush(focusColor(option));
        painter->drawRect(QRectF(rect.x() + r, rect.y(),
                                  rect.width() - 2 * r, 2));
    }

    painter->restore();
}

void BreezeStyle::drawMenuItem(QPainter *painter,
                                const QStyleOption *option,
                                const QWidget *widget) const
{
    Q_UNUSED(widget);
    const QStyleOptionMenuItem *menuOption =
        qstyleoption_cast<const QStyleOptionMenuItem *>(option);
    if (!menuOption)
        return;

    QRectF rect = QRectF(option->rect);

    /* Separator */
    if (menuOption->menuItemType == QStyleOptionMenuItem::Separator) {
        painter->save();
        QColor sep = separatorColor(option);
        painter->setPen(QPen(sep, 1.0));
        qreal y = rect.center().y();
        painter->drawLine(QPointF(rect.x() + Metrics::MenuSeparatorMargin, y),
                          QPointF(rect.right() - Metrics::MenuSeparatorMargin, y));
        painter->restore();
        return;
    }

    /* Hover highlight */
    if (option->state & QStyle::State_Selected) {
        drawRoundedRect(painter, rect.adjusted(4, 1, -4, -1),
                        Metrics::FrameRadius,
                        hoverColor(option));
    }
}

void BreezeStyle::drawHeaderSection(QPainter *painter,
                                     const QStyleOption *option,
                                     const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect);

    QColor bg = option->palette.color(QPalette::Button);
    if (option->state & QStyle::State_MouseOver)
        bg = bg.lighter(110);

    painter->save();
    painter->setPen(Qt::NoPen);
    painter->setBrush(bg);
    painter->drawRect(rect);

    /* Bottom border */
    painter->setPen(QPen(separatorColor(option), 1.0));
    painter->drawLine(QPointF(rect.x(), rect.bottom()),
                      QPointF(rect.right(), rect.bottom()));

    painter->restore();
}

/* ========================================================================= */
/* Complex controls                                                          */
/* ========================================================================= */

void BreezeStyle::drawComplexControl(ComplexControl control,
                                      const QStyleOptionComplex *option,
                                      QPainter *painter,
                                      const QWidget *widget) const
{
    switch (control) {
    case CC_ScrollBar:
        drawScrollBar(painter, option, widget);
        return;

    case CC_Slider:
        drawSlider(painter, option, widget);
        return;

    case CC_ComboBox:
        drawComboBox(painter, option, widget);
        return;

    case CC_SpinBox:
        drawSpinBox(painter, option, widget);
        return;

    case CC_ToolButton:
        drawToolButton(painter, option, widget);
        return;

    case CC_GroupBox:
        drawGroupBox(painter, option, widget);
        return;

    default:
        QProxyStyle::drawComplexControl(control, option, painter, widget);
        return;
    }
}

void BreezeStyle::drawScrollBar(QPainter *painter,
                                 const QStyleOptionComplex *option,
                                 const QWidget *widget) const
{
    Q_UNUSED(widget);
    const QStyleOptionSlider *sliderOption =
        qstyleoption_cast<const QStyleOptionSlider *>(option);
    if (!sliderOption)
        return;

    /* Groove */
    QRectF grooveRect = QRectF(subControlRect(CC_ScrollBar, option,
                                               SC_ScrollBarGroove, widget));
    drawRoundedRect(painter, grooveRect, Metrics::ScrollBarRadius,
                    option->palette.color(QPalette::Window).darker(105));

    /* Slider handle */
    QRectF sliderRect = QRectF(subControlRect(CC_ScrollBar, option,
                                               SC_ScrollBarSlider, widget));
    QColor handleColor = option->palette.color(QPalette::Mid);
    if (option->state & QStyle::State_MouseOver)
        handleColor = handleColor.lighter(120);
    if (option->state & QStyle::State_Sunken)
        handleColor = focusColor(option);

    /* Inset the handle slightly within the groove */
    sliderRect.adjust(2, 2, -2, -2);
    drawRoundedRect(painter, sliderRect, Metrics::ScrollBarRadius - 2,
                    handleColor);
}

void BreezeStyle::drawSlider(QPainter *painter,
                              const QStyleOptionComplex *option,
                              const QWidget *widget) const
{
    const QStyleOptionSlider *sliderOption =
        qstyleoption_cast<const QStyleOptionSlider *>(option);
    if (!sliderOption)
        return;

    QRectF grooveRect = QRectF(subControlRect(CC_Slider, option,
                                               SC_SliderGroove, widget));
    QRectF handleRect = QRectF(subControlRect(CC_Slider, option,
                                               SC_SliderHandle, widget));

    bool horizontal = (sliderOption->orientation == Qt::Horizontal);

    /* Groove */
    QRectF groove = grooveRect;
    if (horizontal) {
        groove.setHeight(Metrics::SliderGrooveHeight);
        groove.moveCenter(grooveRect.center());
    } else {
        groove.setWidth(Metrics::SliderGrooveHeight);
        groove.moveCenter(grooveRect.center());
    }

    drawRoundedRect(painter, groove, Metrics::SliderGrooveHeight / 2,
                    option->palette.color(QPalette::Window).darker(110));

    /* Filled portion up to handle */
    QRectF filled = groove;
    if (horizontal)
        filled.setWidth(handleRect.center().x() - groove.x());
    else
        filled.setTop(handleRect.center().y());

    if (filled.isValid())
        drawRoundedRect(painter, filled, Metrics::SliderGrooveHeight / 2,
                        focusColor(option));

    /* Handle */
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);

    QColor handleBg = option->palette.color(QPalette::Base);
    QColor handleBorder = separatorColor(option);

    if (option->state & QStyle::State_Sunken) {
        handleBg = focusColor(option);
        handleBorder = focusColor(option).darker(120);
    } else if (option->state & QStyle::State_MouseOver) {
        handleBorder = focusColor(option);
    }

    painter->setPen(QPen(handleBorder, 1.0));
    painter->setBrush(handleBg);
    painter->drawEllipse(handleRect);

    painter->restore();
}

void BreezeStyle::drawComboBox(QPainter *painter,
                                const QStyleOptionComplex *option,
                                const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect).adjusted(1, 1, -1, -1);
    QColor bg = buttonColor(option, widget);
    QColor border = separatorColor(option);

    drawRoundedRect(painter, rect, Metrics::ButtonRadius, bg, border);

    /* Down arrow */
    QRectF arrowRect = QRectF(subControlRect(CC_ComboBox, option,
                                              SC_ComboBoxArrow, widget));
    drawArrow(painter, arrowRect, Qt::DownArrow,
              option->palette.color(QPalette::ButtonText));

    /* Focus ring */
    if (option->state & QStyle::State_HasFocus)
        drawFocusRing(painter, rect, Metrics::ButtonRadius,
                      focusColor(option));
}

void BreezeStyle::drawSpinBox(QPainter *painter,
                               const QStyleOptionComplex *option,
                               const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect).adjusted(1, 1, -1, -1);
    QColor bg = option->palette.color(QPalette::Base);
    QColor border = separatorColor(option);

    drawRoundedRect(painter, rect, Metrics::ButtonRadius, bg, border);

    /* Up/down buttons */
    QRectF upRect = QRectF(subControlRect(CC_SpinBox, option,
                                           SC_SpinBoxUp, widget));
    QRectF downRect = QRectF(subControlRect(CC_SpinBox, option,
                                             SC_SpinBoxDown, widget));

    drawArrow(painter, upRect, Qt::UpArrow,
              option->palette.color(QPalette::ButtonText));
    drawArrow(painter, downRect, Qt::DownArrow,
              option->palette.color(QPalette::ButtonText));

    /* Focus ring */
    if (option->state & QStyle::State_HasFocus)
        drawFocusRing(painter, rect, Metrics::ButtonRadius,
                      focusColor(option));
}

void BreezeStyle::drawToolButton(QPainter *painter,
                                  const QStyleOptionComplex *option,
                                  const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect);

    /* Only draw background on hover or press */
    if (option->state & (QStyle::State_MouseOver | QStyle::State_Sunken)) {
        QColor bg = buttonColor(option, widget);
        drawRoundedRect(painter, rect.adjusted(1, 1, -1, -1),
                        Metrics::ButtonRadius, bg);
    }

    /* Delegate label/icon drawing to proxy */
    QProxyStyle::drawComplexControl(CC_ToolButton, option, painter, widget);
}

void BreezeStyle::drawGroupBox(QPainter *painter,
                                const QStyleOptionComplex *option,
                                const QWidget *widget) const
{
    Q_UNUSED(widget);
    QRectF rect = QRectF(option->rect);
    QColor border = separatorColor(option);

    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);
    painter->setPen(QPen(border, 1.0));
    painter->setBrush(Qt::NoBrush);
    painter->drawRoundedRect(rect.adjusted(0.5, 12.5, -0.5, -0.5),
                              Metrics::FrameRadius, Metrics::FrameRadius);
    painter->restore();
}

/* ========================================================================= */
/* Sub-control rectangles                                                    */
/* ========================================================================= */

QRect BreezeStyle::subControlRect(ComplexControl control,
                                   const QStyleOptionComplex *option,
                                   SubControl subControl,
                                   const QWidget *widget) const
{
    QRect rect = option->rect;

    switch (control) {
    case CC_ComboBox:
        if (subControl == SC_ComboBoxArrow) {
            int arrowWidth = Metrics::ComboBoxArrowSize + 2 * Metrics::ComboBoxPadding;
            return QRect(rect.right() - arrowWidth, rect.y(),
                         arrowWidth, rect.height());
        }
        if (subControl == SC_ComboBoxEditField) {
            int arrowWidth = Metrics::ComboBoxArrowSize + 2 * Metrics::ComboBoxPadding;
            return QRect(rect.x() + Metrics::ComboBoxPadding, rect.y(),
                         rect.width() - arrowWidth - Metrics::ComboBoxPadding,
                         rect.height());
        }
        break;

    case CC_SpinBox:
        if (subControl == SC_SpinBoxUp) {
            return QRect(rect.right() - Metrics::SpinBoxArrowSize * 2,
                         rect.y(), Metrics::SpinBoxArrowSize * 2,
                         rect.height() / 2);
        }
        if (subControl == SC_SpinBoxDown) {
            return QRect(rect.right() - Metrics::SpinBoxArrowSize * 2,
                         rect.y() + rect.height() / 2,
                         Metrics::SpinBoxArrowSize * 2, rect.height() / 2);
        }
        if (subControl == SC_SpinBoxEditField) {
            return QRect(rect.x() + Metrics::ComboBoxPadding, rect.y(),
                         rect.width() - Metrics::SpinBoxArrowSize * 2 - Metrics::ComboBoxPadding,
                         rect.height());
        }
        break;

    default:
        break;
    }

    return QProxyStyle::subControlRect(control, option, subControl, widget);
}

/* ========================================================================= */
/* Size hints                                                                */
/* ========================================================================= */

QSize BreezeStyle::sizeFromContents(ContentsType type,
                                     const QStyleOption *option,
                                     const QSize &size,
                                     const QWidget *widget) const
{
    QSize s = QProxyStyle::sizeFromContents(type, option, size, widget);

    switch (type) {
    case CT_PushButton:
        s = s.expandedTo(QSize(Metrics::ButtonMinWidth, Metrics::ButtonMinHeight));
        s += QSize(2 * Metrics::ButtonPadding, 0);
        break;

    case CT_CheckBox:
    case CT_RadioButton:
        s.setHeight(qMax(s.height(), Metrics::CheckBoxSize));
        s += QSize(Metrics::CheckBoxSpacing, 0);
        break;

    case CT_MenuItem:
        s.setHeight(qMax(s.height(), Metrics::MenuItemHeight));
        break;

    case CT_TabBarTab:
        s.setHeight(qMax(s.height(), Metrics::TabBarHeight));
        s += QSize(2 * Metrics::TabBarPadding, 0);
        break;

    case CT_ComboBox:
        s.setHeight(qMax(s.height(), Metrics::ComboBoxMinHeight));
        break;

    case CT_SpinBox:
        s.setHeight(qMax(s.height(), Metrics::SpinBoxMinHeight));
        break;

    case CT_ToolButton:
        s = s.expandedTo(QSize(Metrics::ToolButtonMinWidth,
                               Metrics::ToolButtonMinWidth));
        break;

    default:
        break;
    }

    return s;
}

/* ========================================================================= */
/* Pixel metrics                                                             */
/* ========================================================================= */

int BreezeStyle::pixelMetric(PixelMetric metric,
                              const QStyleOption *option,
                              const QWidget *widget) const
{
    switch (metric) {
    case PM_LayoutTopMargin:    return Metrics::LayoutTopMargin;
    case PM_LayoutBottomMargin: return Metrics::LayoutBottomMargin;
    case PM_LayoutLeftMargin:   return Metrics::LayoutLeftMargin;
    case PM_LayoutRightMargin:  return Metrics::LayoutRightMargin;
    case PM_LayoutHorizontalSpacing:
    case PM_LayoutVerticalSpacing:
        return Metrics::LayoutSpacing;

    case PM_DefaultFrameWidth:  return Metrics::FrameWidth;
    case PM_ScrollBarExtent:    return Metrics::ScrollBarWidth;
    case PM_SliderThickness:    return Metrics::SliderHandleSize;
    case PM_SliderLength:       return Metrics::SliderHandleSize;
    case PM_IndicatorWidth:
    case PM_IndicatorHeight:
    case PM_ExclusiveIndicatorWidth:
    case PM_ExclusiveIndicatorHeight:
        return Metrics::CheckBoxSize;

    case PM_MenuHMargin:
    case PM_MenuVMargin:        return 4;
    case PM_ToolBarIconSize:    return 22;
    case PM_SmallIconSize:      return 16;
    case PM_LargeIconSize:      return 48;

    case PM_FocusFrameVMargin:
    case PM_FocusFrameHMargin:  return 2;

    case PM_ToolTipLabelFrameWidth: return Metrics::TooltipPadding;

    default:
        return QProxyStyle::pixelMetric(metric, option, widget);
    }
}

/* ========================================================================= */
/* Style hints                                                               */
/* ========================================================================= */

int BreezeStyle::styleHint(StyleHint hint,
                            const QStyleOption *option,
                            const QWidget *widget,
                            QStyleHintReturn *returnData) const
{
    switch (hint) {
    case SH_RubberBand_Mask:             return false;
    case SH_ToolTip_Mask:                return false;
    case SH_Menu_Mask:                   return false;
    case SH_Widget_Animate:              return true;
    case SH_ScrollBar_ContextMenu:       return true;
    case SH_ItemView_ActivateItemOnSingleClick: return false;
    case SH_DialogButtonLayout:          return QDialogButtonBox::KdeLayout;
    case SH_ToolButtonStyle:             return Qt::ToolButtonTextBesideIcon;
    case SH_FormLayoutWrapPolicy:        return QFormLayout::DontWrapRows;

    default:
        return QProxyStyle::styleHint(hint, option, widget, returnData);
    }
}

/* ========================================================================= */
/* Sub-element rectangles                                                    */
/* ========================================================================= */

QRect BreezeStyle::subElementRect(SubElement element,
                                   const QStyleOption *option,
                                   const QWidget *widget) const
{
    return QProxyStyle::subElementRect(element, option, widget);
}

/* ========================================================================= */
/* Standard icons                                                            */
/* ========================================================================= */

QIcon BreezeStyle::standardIcon(StandardPixmap standardIcon,
                                 const QStyleOption *option,
                                 const QWidget *widget) const
{
    /* Delegate to Breeze icon theme.  Qt will look up the icon by name
     * from the configured icon theme directory. */
    return QProxyStyle::standardIcon(standardIcon, option, widget);
}

/* ========================================================================= */
/* Widget polishing (animation registration)                                 */
/* ========================================================================= */

void BreezeStyle::polish(QWidget *widget)
{
    QProxyStyle::polish(widget);

    /* Register widgets that should have hover/focus animations */
    if (qobject_cast<QPushButton *>(widget) ||
        qobject_cast<QToolButton *>(widget) ||
        qobject_cast<QCheckBox *>(widget) ||
        qobject_cast<QRadioButton *>(widget) ||
        qobject_cast<QComboBox *>(widget) ||
        qobject_cast<QSpinBox *>(widget) ||
        qobject_cast<QSlider *>(widget) ||
        qobject_cast<QScrollBar *>(widget) ||
        qobject_cast<QLineEdit *>(widget) ||
        qobject_cast<QTabBar *>(widget)) {

        widget->setAttribute(Qt::WA_Hover, true);
        m_animationEngine->registerWidget(widget);
        widget->installEventFilter(this);
    }

    /* Enable mouse tracking for scroll areas */
    if (auto *scrollArea = qobject_cast<QAbstractScrollArea *>(widget))
        scrollArea->setMouseTracking(true);
}

void BreezeStyle::unpolish(QWidget *widget)
{
    m_animationEngine->unregisterWidget(widget);
    widget->removeEventFilter(this);
    QProxyStyle::unpolish(widget);
}

void BreezeStyle::polish(QApplication *app)
{
    QProxyStyle::polish(app);
}

void BreezeStyle::polish(QPalette &palette)
{
    QProxyStyle::polish(palette);
}

/* ========================================================================= */
/* Event filter for animation updates                                        */
/* ========================================================================= */

bool BreezeStyle::eventFilter(QObject *obj, QEvent *event)
{
    QWidget *widget = qobject_cast<QWidget *>(obj);
    if (!widget)
        return QProxyStyle::eventFilter(obj, event);

    switch (event->type()) {
    case QEvent::HoverEnter:
        m_animationEngine->updateHover(widget, true);
        break;
    case QEvent::HoverLeave:
        m_animationEngine->updateHover(widget, false);
        break;
    case QEvent::FocusIn:
        m_animationEngine->updateFocus(widget, true);
        break;
    case QEvent::FocusOut:
        m_animationEngine->updateFocus(widget, false);
        break;
    default:
        break;
    }

    return QProxyStyle::eventFilter(obj, event);
}

/* ========================================================================= */
/* Plugin factory                                                            */
/* ========================================================================= */

QStyle *BreezeStylePlugin::create(const QString &key)
{
    if (key.compare(QStringLiteral("breeze"), Qt::CaseInsensitive) == 0)
        return new BreezeStyle();
    return nullptr;
}

QStringList BreezeStylePlugin::keys() const
{
    return QStringList{QStringLiteral("breeze")};
}

} /* namespace Breeze */
